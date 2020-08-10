use futures::stream::TryStreamExt;
use macaddr::MacAddr;
use rtnetlink::Handle;
use std::{
    str::FromStr,
    sync::atomic::{AtomicUsize, Ordering},
    time::Duration,
};
use tokio::time;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

struct VethLink<'a> {
    handle: Handle,
    if_index: u32,
    if_name: &'a str,
    peer_name: &'a str,
    if_addr: &'a str,
    peer_addr: &'a str,
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

async fn delete_link(veth_link: VethLink<'_>) -> anyhow::Result<()> {
    Ok(veth_link
        .handle
        .link()
        .del(veth_link.if_index)
        .execute()
        .await?)
}

async fn build_veth_link<'a>(
    if_name: &'a str,
    peer_name: &'a str,
    if_addr: &'a str,
    peer_addr: &'a str,
) -> anyhow::Result<VethLink<'a>> {
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

    Ok(VethLink {
        handle,
        if_index,
        if_name,
        peer_name,
        if_addr,
        peer_addr,
    })
}

async fn set_up_veth_link(veth_link: &VethLink<'_>) -> anyhow::Result<()> {
    let peer_index = get_link_index(&veth_link.handle, veth_link.peer_name).await?;

    set_link_up(&veth_link.handle, veth_link.if_index).await?;
    set_link_up(&veth_link.handle, peer_index).await?;

    //set_link_addr(&veth_link.handle, veth_link.if_index, veth_link.if_addr).await?;
    //set_link_addr(&veth_link.handle, peer_index, veth_link.peer_addr).await?;

    Ok(())
}

fn generate_random_bytes(len: u32) -> Vec<u8> {
    (0..len).map(|_| rand::random::<u8>()).collect()
}

fn generate_mac_addr() -> String {
    let bytes = generate_random_bytes(6);

    format!(
        "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
    )
}

#[tokio::test]
async fn test_setup() {
    let ctr = COUNTER.fetch_add(1, Ordering::SeqCst);

    let if_name = format!("test{}", ctr);
    let if_addr = generate_mac_addr();

    let peer_name = format!("echo{}", ctr);
    let peer_addr = generate_mac_addr();

    println!("if_addr: {}", if_addr);
    println!("peer addr: {}", peer_addr);

    let veth_link = build_veth_link(&if_name, &peer_name, &if_addr, &peer_addr)
        .await
        .unwrap();

    if let Err(e) = set_up_veth_link(&veth_link).await {
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
