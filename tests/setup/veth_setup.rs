use futures::stream::TryStreamExt;
use rtnetlink::Handle;
use std::net::{IpAddr, Ipv4Addr};
use tokio::{runtime, task};

#[derive(Debug, Clone, Copy)]
pub enum LinkStatus {
    Up,
    Down,
}

pub struct VethDev {
    handle: Handle,
    index: u32,
    if_name: String,
}

impl VethDev {
    pub fn if_name(&self) -> &str {
        &self.if_name
    }

    async fn set_status(&self, status: LinkStatus) -> anyhow::Result<()> {
        Ok(match status {
            LinkStatus::Up => {
                self.handle.link().set(self.index).up().execute().await?;
            }
            LinkStatus::Down => {
                self.handle.link().set(self.index).down().execute().await?;
            }
        })
    }

    async fn set_addr(&self, addr: Vec<u8>) -> anyhow::Result<()> {
        self.handle
            .link()
            .set(self.index)
            .address(addr)
            .execute()
            .await?;

        Ok(())
    }

    async fn set_ip_addr(&self, ip_addr: LinkIpAddr) -> anyhow::Result<()> {
        self.handle
            .address()
            .add(
                self.index,
                IpAddr::V4(ip_addr.addr.clone()),
                ip_addr.prefix_len,
            )
            .execute()
            .await?;

        Ok(())
    }
}

pub struct VethPair {
    dev1: VethDev,
    dev2: VethDev,
}

impl VethPair {
    pub async fn set_status(&self, status: LinkStatus) -> anyhow::Result<()> {
        for dev in [&self.dev1, &self.dev2] {
            dev.set_status(status).await?;
        }
        Ok(())
    }

    pub fn dev1(&self) -> &VethDev {
        &self.dev1
    }

    pub fn dev2(&self) -> &VethDev {
        &self.dev2
    }
}

impl Drop for VethPair {
    fn drop(&mut self) {
        let (handle, index, if_name) = (&self.dev1.handle, self.dev1.index, &self.dev1.if_name);

        let res = task::block_in_place(move || {
            runtime::Handle::current()
                .block_on(async move { handle.link().del(index).execute().await })
        });

        if let Err(e) = res {
            eprintln!("failed to delete link: {:?} (you may need to delete it manually with 'sudo ip link del {}')", e, if_name);
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct LinkIpAddr {
    addr: Ipv4Addr,
    prefix_len: u8,
}

impl LinkIpAddr {
    pub fn new(addr: Ipv4Addr, prefix_len: u8) -> Self {
        LinkIpAddr { addr, prefix_len }
    }

    pub fn octets(&self) -> [u8; 4] {
        self.addr.octets()
    }
}

#[derive(Clone, Debug)]
pub struct VethDevConfig {
    if_name: String,
    addr: Option<[u8; 6]>,
    ip_addr: Option<LinkIpAddr>,
}

impl VethDevConfig {
    pub fn new(if_name: String, addr: Option<[u8; 6]>, ip_addr: Option<LinkIpAddr>) -> Self {
        Self {
            if_name,
            addr,
            ip_addr,
        }
    }

    pub fn if_name(&self) -> &str {
        &self.if_name
    }

    pub fn addr(&self) -> Option<[u8; 6]> {
        self.addr
    }

    pub fn ip_addr(&self) -> Option<LinkIpAddr> {
        self.ip_addr
    }
}

async fn get_link_index(handle: &Handle, name: &str) -> anyhow::Result<u32> {
    Ok(handle
        .link()
        .get()
        .match_name(name.into())
        .execute()
        .try_next()
        .await?
        .expect(format!("no link with name {} found", name).as_str())
        .header
        .index)
}

pub async fn build_veth_pair(
    dev1_config: &VethDevConfig,
    dev2_config: &VethDevConfig,
) -> anyhow::Result<VethPair> {
    let (connection, handle, _) = rtnetlink::new_connection().unwrap();

    tokio::spawn(connection);

    handle
        .link()
        .add()
        .veth(dev1_config.if_name.clone(), dev2_config.if_name.clone())
        .execute()
        .await?;

    let dev1_index = get_link_index(&handle, &dev1_config.if_name).await.expect(
        format!(
            "failed to retrieve index for dev1, delete link manually: 'sudo ip link del {}'",
            dev1_config.if_name
        )
        .as_str(),
    );

    let dev2_index = get_link_index(&handle, &dev2_config.if_name).await.expect(
        format!(
            "failed to retrieve index for dev2, delete link manually: 'sudo ip link del {}'",
            dev1_config.if_name
        )
        .as_str(),
    );

    let veth_pair = VethPair {
        dev1: VethDev {
            handle: handle.clone(),
            index: dev1_index,
            if_name: dev1_config.if_name.clone(),
        },
        dev2: VethDev {
            handle: handle.clone(),
            index: dev2_index,
            if_name: dev2_config.if_name.clone(),
        },
    };

    for (d, c) in [
        (&veth_pair.dev1, dev1_config),
        (&veth_pair.dev2, dev2_config),
    ] {
        if let Some(addr) = c.addr {
            d.set_addr(addr.into()).await?;
        }
        if let Some(ip_addr) = c.ip_addr {
            d.set_ip_addr(ip_addr).await?;
        }
    }

    Ok(veth_pair)
}

pub async fn run_with_veth_pair<F>(
    f: F,
    dev1_config: VethDevConfig,
    dev2_config: VethDevConfig,
) -> anyhow::Result<()>
where
    F: FnOnce(VethDevConfig, VethDevConfig) + Send + 'static,
{
    let veth_pair = build_veth_pair(&dev1_config, &dev2_config).await.unwrap();

    veth_pair.set_status(LinkStatus::Up).await?;

    let res = task::spawn_blocking(move || f(dev1_config, dev2_config)).await;

    veth_pair.set_status(LinkStatus::Down).await?;

    Ok(res?)
}
