use futures::stream::TryStreamExt;
use rtnetlink::Handle;
use std::{
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::time;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct VethLink {
    handle: Handle,
    if_index: u32,
}

async fn get_link_index(handle: &Handle, name: &str) -> anyhow::Result<u32> {
    Ok(handle
        .link()
        .get()
        .set_name_filter(name.into())
        .execute()
        .try_next()
        .await?
        .expect(format!("No link with name {} found", name).as_str())
        .header
        .index)
}

async fn set_link_up(handle: &Handle, index: u32) -> anyhow::Result<()> {
    Ok(handle.link().set(index).up().execute().await?)
}

async fn delete_link(veth_link: VethLink) -> anyhow::Result<()> {
    Ok(veth_link
        .handle
        .link()
        .del(veth_link.if_index)
        .execute()
        .await?)
}

async fn build_veth_link(if_name: &str, peer_name: &str) -> anyhow::Result<VethLink> {
    let (connection, handle, _) = rtnetlink::new_connection().unwrap();

    tokio::spawn(connection);

    handle
        .link()
        .add()
        .veth(if_name.into(), peer_name.into())
        .execute()
        .await?;

    let if_index = get_link_index(&handle, if_name).await.expect(
        format!(
            "Failed to retrieve index. Remember to remove link manually: 'sudo ip link del {}'",
            if_name
        )
        .as_str(),
    );

    Ok(VethLink { handle, if_index })
}

async fn set_up_veth_link(veth_link: &VethLink, peer_name: &str) -> anyhow::Result<()> {
    let peer_index = get_link_index(&veth_link.handle, peer_name).await?;

    set_link_up(&veth_link.handle, veth_link.if_index).await?;
    set_link_up(&veth_link.handle, peer_index).await?;

    Ok(())
}

#[tokio::test]
async fn test_setup() {
    let ctr = COUNTER.fetch_add(1, Ordering::SeqCst);

    let if_name = format!("test{}", ctr);
    let peer_name = format!("echo{}", ctr);

    let veth_link = build_veth_link(&if_name, &peer_name).await.unwrap();

    if let Err(e) = set_up_veth_link(&veth_link, &peer_name).await {
        eprintln!("Error setting up veth link: {}", e);
        delete_link(veth_link)
            .await
            .expect("Error deleting test link");
        return;
    }

    time::delay_for(Duration::from_secs(10)).await;

    delete_link(veth_link)
        .await
        .expect("Error deleting test link");
}
