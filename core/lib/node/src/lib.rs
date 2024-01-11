use std::{any::Any, cell::RefCell, collections::HashMap, fmt};

use futures::{future::BoxFuture, FutureExt};
use resources::{stop_receiver::StopReceiverResource, Resource};
use tasks::TaskInitError;
use tokio::{runtime::Runtime, sync::watch};
// Public re-exports from external crate to minimize the required dependencies.
pub use zksync_health_check::{CheckHealth, ReactiveHealthCheck};

pub mod resources;
pub mod tasks;

pub trait IntoZkSyncTask: 'static + Send + Sync {
    type Config: 'static + Send + Sync;

    /// Creates a new task.
    /// Normally, at this step the task is only expected to gather required resources from `ZkSyncNode`.
    ///
    /// If additional preparations are required, they should be done in `before_launch`.
    fn create(
        node: &ZkSyncNode,
        config: Self::Config,
    ) -> Result<Box<dyn ZkSyncTask>, tasks::TaskInitError>;
}

/// A task represents some code that "runs".
/// During its creation, it uses its own config and resources added to the `ZkSyncNode`.
///
/// TODO more elaborate docs
#[async_trait::async_trait]
pub trait ZkSyncTask: 'static + Send + Sync {
    /// Gets the healthcheck for the task, if it exists.
    /// Guaranteed to be called only once per task.
    fn healtcheck(&mut self) -> Option<Box<dyn CheckHealth>>;

    /// Runs the task.
    ///
    /// Once any of the task returns, the node will shutdown.
    /// If the task returns an error, the node will spawn an error-level log message and will return a non-zero exit code.
    async fn run(self: Box<Self>) -> anyhow::Result<()>;

    /// TODO: elaborate on sharing state with the hook.
    fn after_finish(&self) -> Option<BoxFuture<'static, ()>> {
        None
    }

    /// TODO: elaborate on sharing state with the hook.
    ///
    /// Asynchronous hook that will be called after *each task* has finished their cleanup.
    /// It is guaranteed that no other task is running at this point, e.g. `ZkSyncNode` will invoke
    /// this hook sequentially for each task.
    ///
    /// This hook can be used to perform some cleanup that assumes exclusive access to the resources, e.g.
    /// to rollback some state.
    fn after_node_shutdown(&self) -> Option<BoxFuture<'static, ()>> {
        None
    }
}

struct TaskRepr {
    name: String,
    task: Option<BoxFuture<'static, anyhow::Result<()>>>,
    after_finish: Option<BoxFuture<'static, ()>>,
    after_node_shutdown: Option<BoxFuture<'static, ()>>,
}

impl fmt::Debug for TaskRepr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TaskRepr")
            .field("name", &self.name)
            .finish_non_exhaustive()
    }
}

pub trait ResourceProvider: 'static + Send + Sync + fmt::Debug {
    /// Resources are expected to be available.
    /// In case it isn't possible to obtain the resource, the provider is free to either return `None`
    /// (if it assumes that the node can continue without this resource), or to panic.
    fn get_resource(&self, name: &str) -> Option<Box<dyn Any>>;
}

/// "Manager" class of the node. Collects all the resources and tasks,
/// then runs tasks until completion.
pub struct ZkSyncNode {
    resource_provider: Box<dyn ResourceProvider>,
    /// Anything that may be needed by 1 or more tasks to be created.
    /// Think `ConnectionPool`, `TxSender`, `SealManager`, `EthGateway`, `GasAdjuster`.
    /// There is a note on `dyn Any` below the snippet.
    resources: RefCell<HashMap<String, Box<dyn Any>>>,
    /// Named tasks.
    tasks: Vec<TaskRepr>,
    // TODO: healthchecks

    // TODO: circuit breakers
    stop_sender: watch::Sender<bool>,
    runtime: Runtime,
}

impl ZkSyncNode {
    pub fn new<R: ResourceProvider>(resource_provider: R) -> anyhow::Result<Self> {
        if tokio::runtime::Handle::try_current().is_ok() {
            anyhow::bail!(
                "Detected a Tokio Runtime. ZkSyncNode manages its own runtime and does not support nested runtimes"
            );
        }
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();

        let (stop_sender, stop_receiver) = watch::channel(false);
        let mut self_ = Self {
            resource_provider: Box::new(resource_provider),
            resources: RefCell::default(),
            tasks: Vec::new(),
            stop_sender,
            runtime,
        };
        self_.add_resource(
            StopReceiverResource::RESOURCE_NAME,
            StopReceiverResource::new(stop_receiver),
        );

        Ok(self_)
    }

    /// Provides access to the runtime used by the node.
    /// Can be used to execute non-blocking code in the task constructors, or to spawn additional tasks within
    /// the same runtime.
    /// If some tasks stores the handle to spawn additional tasks, it is considered responsible for all the
    /// required cleanup.
    pub fn runtime_handle(&self) -> &tokio::runtime::Handle {
        self.runtime.handle()
    }

    /// Adds a resource. By default, any resource can be requested by multiple
    /// components, thus `T: Clone`. Think `Arc<U>`.
    fn add_resource<T: Resource>(&mut self, name: impl AsRef<str>, resource: T) {
        self.resources
            .borrow_mut()
            .insert(name.as_ref().into(), Box::new(resource));
    }

    /// To be called in `ZkSyncTask::new(..)`.
    pub fn get_resource<T: Resource>(&self, name: impl AsRef<str>) -> Option<T> {
        let downcast_clone = |resource: &Box<dyn Any>| {
            resource
                .downcast_ref::<T>()
                .unwrap_or_else(|| {
                    panic!(
                        "Resource {} is not of type {}",
                        name.as_ref(),
                        std::any::type_name::<T>()
                    )
                })
                .clone()
        };

        let name = name.as_ref();
        // Check whether the resource is already available.
        if let Some(resource) = self.resources.borrow().get(name) {
            return Some(downcast_clone(resource));
        }

        // Try to fetch the resource from the provider.
        if let Some(resource) = self.resource_provider.get_resource(name) {
            // First, ensure the type matches.
            let downcasted = downcast_clone(&resource);
            // Then, add it to the local resources.
            self.resources.borrow_mut().insert(name.into(), resource);
            return Some(downcasted);
        }

        // No such resource.
        // The requester is allowed to decide whether this is an error or not.
        None
    }

    /// Takes care of task creation.
    /// May do some "registration" stuff for reporting purposes.
    pub fn add_task<F: FnOnce(&ZkSyncNode) -> Result<Box<dyn ZkSyncTask>, TaskInitError>>(
        &mut self,
        name: impl AsRef<str>,
        task_constructor: F,
    ) -> &mut Self {
        let task = task_constructor(self).unwrap(); // TODO: Do not unwrap
        let after_finish = task.after_finish();
        let after_node_shutdown = task.after_node_shutdown();
        let name = String::from(name.as_ref());
        let task_future = Box::pin(task.run());
        let task_repr = TaskRepr {
            name,
            task: Some(task_future),
            after_finish,
            after_node_shutdown,
        };
        self.tasks.push(task_repr);
        self
    }

    /// Runs the system.
    pub fn run(mut self) -> anyhow::Result<()> {
        if self.tasks.is_empty() {
            anyhow::bail!("No tasks to run");
        }

        let rt_handle = self.runtime.handle().clone();
        let join_handles: Vec<_> = self
            .tasks
            .iter_mut()
            .map(|task| {
                let task = task.task.take().expect(
                    "Tasks are created by the node and must be Some prior to calling this method",
                );
                rt_handle.spawn(task).fuse()
            })
            .collect();

        // Run the tasks until one of them exits.
        let (resolved, idx, remaining) = self
            .runtime
            .block_on(futures::future::select_all(join_handles));
        let task_name = &self.tasks[idx].name;
        let failure = match resolved {
            Ok(Ok(())) => {
                tracing::info!("Task {task_name} completed");
                false
            }
            Ok(Err(err)) => {
                tracing::error!("Task {task_name} exited with an error: {err}");
                true
            }
            Err(_) => {
                tracing::error!("Task {task_name} panicked");
                true
            }
        };

        // Send stop signal to remaining tasks and wait for them to finish.
        // Given that we are shutting down, we do not really care about returned values.
        self.stop_sender.send(true).ok();
        self.runtime.block_on(futures::future::join_all(remaining));

        // Call after_finish hooks.
        // For this and shutdown hooks we need to use local set, since these futures are not `Send`.
        let local_set = tokio::task::LocalSet::new();
        let join_handles = self.tasks.iter_mut().filter_map(|task| {
            task.after_finish
                .take()
                .map(|task| local_set.spawn_local(task))
        });
        local_set.block_on(&self.runtime, futures::future::join_all(join_handles));

        // Call after_node_shutdown hooks.
        let join_handles = self.tasks.iter_mut().filter_map(|task| {
            task.after_node_shutdown
                .take()
                .map(|task| local_set.spawn_local(task))
        });
        local_set.block_on(&self.runtime, futures::future::join_all(join_handles));

        if failure {
            anyhow::bail!("Task failed"); // TODO elaborate
        } else {
            Ok(())
        }
    }
}
