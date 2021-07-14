use anyhow::{anyhow, Context};
use futures::stream::TryStreamExt;
use rtnetlink::Handle;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::{
    sync::mpsc::{self, UnboundedReceiver},
    task,
};

static COUNTER: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone)]
struct IfNames {
    dev1: String,
    dev2: String,
}

struct LinkIndices {
    dev1: u32,
    dev2: u32,
}

fn _ctrl_channel() -> Result<UnboundedReceiver<()>, ctrlc::Error> {
    let (tx, rx) = mpsc::unbounded_channel();
    ctrlc::set_handler(move || {
        let _ = tx.send(());
    })?;

    Ok(rx)
}

async fn get_link_index(conn_handle: &Handle, name: &str) -> anyhow::Result<u32> {
    conn_handle
        .link()
        .get()
        .set_name_filter(name.into())
        .execute()
        .try_next()
        .await?
        .map(|link| link.header.index)
        .ok_or(anyhow!("no index found for link {}", name))
}

async fn delete_link(conn_handle: &Handle, index: u32) -> anyhow::Result<()> {
    Ok(conn_handle.link().del(index).execute().await?)
}

async fn set_veth_link_up(conn_handle: &Handle, link_indices: &LinkIndices) -> anyhow::Result<()> {
    conn_handle
        .link()
        .set(link_indices.dev1)
        .up()
        .execute()
        .await?;

    conn_handle
        .link()
        .set(link_indices.dev2)
        .up()
        .execute()
        .await?;

    Ok(())
}

async fn get_link_indices(conn_handle: &Handle, if_names: &IfNames) -> anyhow::Result<LinkIndices> {
    let dev1_index = get_link_index(&conn_handle, &if_names.dev1).await?;
    let dev2_index = get_link_index(&conn_handle, &if_names.dev2).await?;

    Ok(LinkIndices {
        dev1: dev1_index,
        dev2: dev2_index,
    })
}

async fn add_veth_pair(conn_handle: &Handle, if_names: &IfNames) -> anyhow::Result<()> {
    conn_handle
        .link()
        .add()
        .veth(if_names.dev1.clone(), if_names.dev2.clone())
        .execute()
        .await?;

    Ok(())
}

fn create_rtnetlink_connection() -> anyhow::Result<Handle> {
    let (connection, handle, _) = rtnetlink::new_connection()?;

    tokio::spawn(connection);

    Ok(handle)
}

pub async fn run_with_dev<F>(f: F) -> anyhow::Result<()>
where
    F: FnOnce(String, String) + Send + 'static,
{
    let ctr = COUNTER.fetch_add(1, Ordering::SeqCst);

    let if_names = IfNames {
        dev1: format!("xsk_test_dev1_{}", ctr),
        dev2: format!("xsk_test_dev2_{}", ctr),
    };

    let conn_handle =
        create_rtnetlink_connection().with_context(|| "failed to create RTNETLINK connection")?;

    add_veth_pair(&conn_handle, &if_names)
        .await
        .with_context(|| {
            format!(
                "failed to add veth pair {} and {}",
                if_names.dev1, if_names.dev2
            )
        })?;

    let link_indices = get_link_indices(&conn_handle, &if_names)
        .await
        .with_context(|| {
            format!(
                r#"
failed to retrieve link indices
you may need to delete the link manually: 'sudo ip link del {}'
"#,
                if_names.dev1
            )
        })?;

    let mut res = run_with_dev_inner(f, &conn_handle, &link_indices, if_names.clone()).await;

    if let Err(e) = delete_link(&conn_handle, link_indices.dev1).await {
        res = res.with_context(|| {
            format!(
                r#"
failed to delete link: {}
you may need to delete the link manually: 'sudo ip link del {}'
"#,
                e, if_names.dev1
            )
        })
    }

    res
}

async fn run_with_dev_inner<F>(
    f: F,
    conn_handle: &Handle,
    link_indices: &LinkIndices,
    if_names: IfNames,
) -> anyhow::Result<()>
where
    F: FnOnce(String, String) + Send + 'static,
{
    set_veth_link_up(&conn_handle, &link_indices)
        .await
        .with_context(|| "failed to set veth link up")?;

    task::spawn_blocking(move || f(if_names.dev1, if_names.dev2))
        .await
        .map_err(|e| e.into())
}
