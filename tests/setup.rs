use futures::stream::TryStreamExt;
use macaddr::MacAddr;
use rtnetlink::Handle;
use std::{str::FromStr, time::Duration};
use tokio::time;

struct VethLink {
    handle: Handle,
    index: u32,
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

async fn set_link_addr(handle: &Handle, index: u32, addr: &str) -> anyhow::Result<()> {
    let addr = MacAddr::from_str(addr)?;

    Ok(handle
        .link()
        .set(index)
        .address(addr.as_bytes().to_vec())
        .execute()
        .await?)
}

async fn delete_link(veth_link: VethLink) -> anyhow::Result<()> {
    Ok(veth_link
        .handle
        .link()
        .del(veth_link.index)
        .execute()
        .await?)
}

async fn build_veth_link(
    if_name: &str,
    peer_name: &str,
    if_addr: &str,
    peer_addr: &str,
) -> anyhow::Result<VethLink> {
    let (connection, handle, _) = rtnetlink::new_connection().unwrap();

    tokio::spawn(connection);

    handle
        .link()
        .add()
        .veth(if_name.into(), peer_name.into())
        .execute()
        .await?;

    let if_index = get_link_index(&handle, if_name).await?;

    let peer_index = get_link_index(&handle, peer_name).await?;

    set_link_up(&handle, if_index).await?;
    set_link_up(&handle, peer_index).await?;

    set_link_addr(&handle, if_index, if_addr).await?;
    set_link_addr(&handle, peer_index, peer_addr).await?;

    Ok(VethLink {
        handle,
        index: if_index,
    })
}

#[tokio::test]
async fn test_setup() {
    let veth_link = build_veth_link("test", "echo", "04:04:04:04:04:04", "06:06:06:06:06:06")
        .await
        .unwrap();

    time::delay_for(Duration::from_secs(10)).await;

    delete_link(veth_link)
        .await
        .expect("Error deleting test link");
}
