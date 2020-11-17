use futures::stream::TryStreamExt;
use rtnetlink::Handle;
use std::net::IpAddr;
use tokio::sync::oneshot::{Receiver, Sender};

use super::{LinkIpAddr, VethConfig};

struct VethLink {
    handle: Handle,
    dev1_index: u32,
    dev2_index: u32,
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

async fn set_link_addr(handle: &Handle, index: u32, addr: Vec<u8>) -> anyhow::Result<()> {
    Ok(handle.link().set(index).address(addr).execute().await?)
}

async fn set_link_ip_addr(
    handle: &Handle,
    index: u32,
    link_ip_addr: &LinkIpAddr,
) -> anyhow::Result<()> {
    Ok(handle
        .address()
        .add(
            index,
            IpAddr::V4(link_ip_addr.addr.clone()),
            link_ip_addr.prefix_len,
        )
        .execute()
        .await?)
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
            "Failed to retrieve index for dev1. Remove link manually: 'sudo ip link del {}'",
            dev1_if_name
        )
        .as_str(),
    );

    let dev2_index = get_link_index(&handle, dev2_if_name).await.expect(
        format!(
            "Failed to retrieve index for dev2. Remove link manually: 'sudo ip link del {}'",
            dev1_if_name
        )
        .as_str(),
    );

    Ok(VethLink {
        handle,
        dev1_index,
        dev2_index,
    })
}

async fn configure_veth_link(veth_link: &VethLink, veth_config: &VethConfig) -> anyhow::Result<()> {
    set_link_up(&veth_link.handle, veth_link.dev1_index).await?;
    set_link_up(&veth_link.handle, veth_link.dev2_index).await?;

    set_link_addr(
        &veth_link.handle,
        veth_link.dev1_index,
        veth_config.dev1_addr.to_vec(),
    )
    .await?;

    set_link_addr(
        &veth_link.handle,
        veth_link.dev2_index,
        veth_config.dev2_addr.to_vec(),
    )
    .await?;

    set_link_ip_addr(
        &veth_link.handle,
        veth_link.dev1_index,
        &veth_config.dev1_ip_addr,
    )
    .await?;

    set_link_ip_addr(
        &veth_link.handle,
        veth_link.dev2_index,
        &veth_config.dev2_ip_addr,
    )
    .await?;

    Ok(())
}

pub async fn run_veth_link(
    veth_config: &VethConfig,
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

    let veth_link = build_veth_link(&veth_config.dev1_if_name, &veth_config.dev2_if_name)
        .await
        .unwrap();

    if let Err(e) = configure_veth_link(&veth_link, &veth_config).await {
        eprintln!("Error setting up veth link: {}", e);

        delete_link_with_context(
            &veth_link.handle,
            veth_link.dev1_index,
            &veth_config.dev1_if_name,
        )
        .await;

        return;
    }

    // Let spawning thread know that the link is set up
    if let Err(_) = startup_signal.send(()) {
        // Receiver gone away
        delete_link_with_context(
            &veth_link.handle,
            veth_link.dev1_index,
            &veth_config.dev1_if_name,
        )
        .await;

        return;
    }

    let _ = shutdown_signal.await;

    delete_link_with_context(
        &veth_link.handle,
        veth_link.dev1_index,
        &veth_config.dev1_if_name,
    )
    .await;
}
