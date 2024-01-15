use std::fmt;

use crate::{
    healthcheck::{CheckHealth, IntoHealthCheckTask},
    node::{NodeContext, StopReceiver},
    task::ZkSyncTask,
};
use zksync_config::configs::api::HealthCheckConfig;
use zksync_core::api_server::healthcheck::HealthCheckHandle;

pub struct HealthCheckTask {
    config: HealthCheckConfig,
    healthchecks: Vec<Box<dyn CheckHealth>>,
}

impl fmt::Debug for HealthCheckTask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HealthCheckTask")
            .field("config", &self.config)
            .finish_non_exhaustive()
    }
}

impl IntoHealthCheckTask for HealthCheckTask {
    type Config = HealthCheckConfig;

    fn create(
        _node: &NodeContext<'_>,
        healthchecks: Vec<Box<dyn CheckHealth>>,
        config: Self::Config,
    ) -> Result<Box<dyn super::ZkSyncTask>, super::TaskInitError> {
        let self_ = Self {
            config,
            healthchecks,
        };

        Ok(Box::new(self_))
    }
}

#[async_trait::async_trait]
impl ZkSyncTask for HealthCheckTask {
    fn healtcheck(&mut self) -> Option<Box<dyn zksync_health_check::CheckHealth>> {
        // Not needed for the healtcheck server.
        None
    }

    async fn run(mut self: Box<Self>, mut stop_receiver: StopReceiver) -> anyhow::Result<()> {
        let handle = HealthCheckHandle::spawn_server(self.config.bind_addr(), self.healthchecks);
        stop_receiver.0.changed().await?;
        handle.stop().await;

        Ok(())
    }
}