//! Host unit tests for pure kernel logic (`./scripts/core.sh test-host`).
//! The modules below are the kernel's own files, included by path, and re-exported
//! under the same module paths the kernel uses (`crate::drivers::queue`, ...).

#[allow(dead_code)]
#[path = "../../rust/popcorn_kernel/src/drivers/queue.rs"]
pub mod queue_impl;

#[allow(dead_code)]
#[path = "../../rust/popcorn_kernel/src/drivers/virtio/virtqueue.rs"]
pub mod virtqueue_impl;

#[allow(dead_code)]
#[path = "../../rust/popcorn_kernel/src/util/csum.rs"]
pub mod csum_impl;

/// NetDevice trait + loopback (`drivers/net/mod.rs` pulls in `loopback.rs`).
#[allow(dead_code)]
#[path = "../../rust/popcorn_kernel/src/drivers/net/mod.rs"]
pub mod netdev_impl;

/// Protocol stack (`net/mod.rs` pulls in eth/arp/ipv4/icmp/udp/dhcp/dns/iface).
#[allow(dead_code)]
#[path = "../../rust/popcorn_kernel/src/net/mod.rs"]
pub mod net_impl;

pub mod drivers {
    pub use crate::netdev_impl as net;
    pub use crate::queue_impl as queue;
    pub mod virtio {
        pub use crate::virtqueue_impl as virtqueue;
    }
}

pub mod util {
    pub use crate::csum_impl as csum;
}

pub use crate::net_impl as net;

#[cfg(test)]
mod csum_tests;
#[cfg(test)]
mod net_dhcp_tests;
#[cfg(test)]
mod net_dns_tests;
#[cfg(test)]
mod net_fuzz_tests;
#[cfg(test)]
mod net_proto_fuzz_tests;
#[cfg(test)]
mod net_tests;
#[cfg(test)]
mod net_udp_tests;
#[cfg(test)]
mod net_wire;
#[cfg(test)]
mod virtqueue_tests;
