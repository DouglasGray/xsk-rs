use futures::stream::TryStreamExt;
use rtnetlink::Handle;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::task;
use std::time;
use std::thread::sleep;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

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

async fn set_up_veth_link(veth_link: &VethLink, dev2_if_name: &str) -> anyhow::Result<()> {
    sleep(time::Duration::from_millis(50));
    let peer_index = get_link_index(&veth_link.handle, dev2_if_name).await?;

    set_link_up(&veth_link.handle, veth_link.dev1_index).await?;
    set_link_up(&veth_link.handle, peer_index).await?;

    Ok(())
}

pub async fn run_with_dev<F>(f: F)
where
    F: FnOnce(String, String) + Send + 'static,
{
    let ctr = COUNTER.fetch_add(1, Ordering::SeqCst);

    let dev1_if_name = format!("xsk_test_dev1_{}", ctr);
    let dev2_if_name = format!("xsk_test_dev2_{}", ctr);

    let veth_link = build_veth_link(&dev1_if_name, &dev2_if_name).await.unwrap();

    if let Err(e) = set_up_veth_link(&veth_link, &dev2_if_name).await {
        eprintln!("Error setting up veth link: {}", e);

        delete_link(&veth_link.handle, veth_link.dev1_index)
            .await
            .expect(
                format!(
                    "Failed to delete link. May need to remove manually: 'sudo ip link del {}'",
                    dev1_if_name
                )
                .as_str(),
            );

        return;
    }

    let dev1_if_name_clone = dev1_if_name.clone();
    let dev2_if_name_clone = dev2_if_name.clone();

    let res = task::spawn_blocking(move || f(dev1_if_name_clone, dev2_if_name_clone)).await;

    delete_link(&veth_link.handle, veth_link.dev1_index)
        .await
        .expect(
            format!(
                "Failed to delete link. May need to remove manually: 'sudo ip link del {}'",
                dev1_if_name
            )
            .as_str(),
        );

    res.unwrap()
}
