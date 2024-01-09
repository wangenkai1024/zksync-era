use zksync_node::{
    resources::{self, pools::PoolsResource},
    ZkSyncNode,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let pools: PoolsResource = todo!();

    let node = ZkSyncNode::new().add_resource(resources::pools::RESOURCE_NAME, pools);

    Ok(())
}
