use futures::stream::TryStreamExt;
use rtnetlink::Handle;
use tokio::sync::oneshot::{Receiver, Sender};

struct VethLink {
    handle: Handle,
    dev1_index: u32,
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

async fn build_veth_link(dev1_if_name: &str, dev2_if_name: &str) -> anyhow::Result<VethLink> {
    let (connection, handle, _) = rtnetlink::new_connection().unwrap();

    tokio::spawn(connection);

    handle
        .link()
        .add()
        .veth(dev1_if_name.into(), dev2_if_name.into())
        .execute()
        .await?;

    let dev1_index = get_link_index(&handle, dev1_if_name).await.expect(
        format!(
            "Failed to retrieve index, this is not expected. Remove link manually: 'sudo ip link del {}'",
            dev1_if_name
        )
        .as_str(),
    );

    Ok(VethLink { handle, dev1_index })
}

async fn configure_veth_link(veth_link: &VethLink, dev2_if_name: &str) -> anyhow::Result<()> {
    let peer_index = get_link_index(&veth_link.handle, dev2_if_name).await?;

    set_link_up(&veth_link.handle, veth_link.dev1_index).await?;
    set_link_up(&veth_link.handle, peer_index).await?;

    Ok(())
}

pub async fn run_veth_link(
    dev1_if_name: &str,
    dev2_if_name: &str,
    startup_signal: Sender<()>,
    shutdown_signal: Receiver<()>,
) {
    async fn delete_link_with_context(handle: &Handle, index: u32, if_name: &str) {
        delete_link(handle, index).await.expect(
            format!(
                "Failed to delete link. May need to remove manually: 'sudo ip link del {}'",
                if_name
            )
            .as_str(),
        )
    }

    let veth_link = build_veth_link(&dev1_if_name, &dev2_if_name).await.unwrap();

    if let Err(e) = configure_veth_link(&veth_link, &dev2_if_name).await {
        eprintln!("Error setting up veth link: {}", e);

        delete_link_with_context(&veth_link.handle, veth_link.dev1_index, dev1_if_name).await;

        return;
    }

    // Let spawning thread know that the link is set up
    if let Err(_) = startup_signal.send(()) {
        // Receiver gone away
        delete_link_with_context(&veth_link.handle, veth_link.dev1_index, dev1_if_name).await;

        return;
    }

    let _ = shutdown_signal.await;

    delete_link_with_context(&veth_link.handle, veth_link.dev1_index, dev1_if_name).await;
}
