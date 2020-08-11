use futures::stream::TryStreamExt;
use rtnetlink::Handle;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::task;

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

async fn delete_link(handle: &Handle, index: u32) -> anyhow::Result<()> {
    Ok(handle.link().del(index).execute().await?)
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
            "Failed to retrieve index, this is not expected. Remove link manually: 'sudo ip link del {}'",
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

pub async fn with_dev<F>(f: F)
where
    F: FnOnce(String) + Send + 'static,
{
    let ctr = COUNTER.fetch_add(1, Ordering::SeqCst);

    let if_name = format!("test{}", ctr);
    let peer_name = format!("echo{}", ctr);

    let veth_link = build_veth_link(&if_name, &peer_name).await.unwrap();

    if let Err(e) = set_up_veth_link(&veth_link, &peer_name).await {
        eprintln!("Error setting up veth link: {}", e);

        delete_link(&veth_link.handle, veth_link.if_index)
            .await
            .expect(
                format!(
                    "Failed to delete link. May need to remove manually: 'sudo ip link del {}'",
                    if_name
                )
                .as_str(),
            );

        return;
    }

    let if_name_clone = if_name.clone();

    let res = task::spawn_blocking(move || f(if_name_clone)).await;

    delete_link(&veth_link.handle, veth_link.if_index)
        .await
        .expect(
            format!(
                "Failed to delete link. May need to remove manually: 'sudo ip link del {}'",
                if_name
            )
            .as_str(),
        );

    res.unwrap()
}
