//! Host unit tests for pure kernel logic (`./scripts/core.sh test-host`).
//! The modules below are the kernel's own files, included by path, and re-exported
//! under the same module paths the kernel uses (`crate::drivers::queue`, ...).

// Kernel modules that build `Vec`/`String` name them through `alloc`.
extern crate alloc;

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

/// Plain MSI arithmetic, MHI register decoding, ath11k register facts (pure parts
/// of `drivers/bus` and `drivers/wifi`).
#[allow(dead_code)]
#[path = "../../rust/popcorn_kernel/src/drivers/bus/msi_math.rs"]
pub mod msi_math_impl;
#[allow(dead_code)]
#[path = "../../rust/popcorn_kernel/src/drivers/bus/mhi.rs"]
pub mod mhi_impl;
#[allow(dead_code)]
#[path = "../../rust/popcorn_kernel/src/drivers/bus/iommu_regs.rs"]
pub mod iommu_regs_impl;
#[allow(dead_code)]
#[path = "../../rust/popcorn_kernel/src/drivers/bus/qrtr.rs"]
pub mod qrtr_impl;
#[allow(dead_code)]
#[path = "../../rust/popcorn_kernel/src/drivers/wifi/qmi.rs"]
pub mod qmi_impl;
#[allow(dead_code)]
#[path = "../../rust/popcorn_kernel/src/drivers/wifi/board2.rs"]
pub mod board2_impl;
#[allow(dead_code)]
#[path = "../../rust/popcorn_kernel/src/drivers/wifi/ath11k_hw.rs"]
pub mod ath11k_hw_impl;
#[allow(dead_code)]
#[path = "../../rust/popcorn_kernel/src/drivers/wifi/fw_manifest.rs"]
pub mod fw_manifest_impl;
/// SHA-256 and the read-only FAT32 reader used to load Wi-Fi firmware.
#[allow(dead_code)]
#[path = "../../rust/popcorn_kernel/src/util/sha256.rs"]
pub mod sha256_impl;
#[allow(dead_code)]
#[path = "../../rust/popcorn_kernel/src/fs/fat_ro.rs"]
pub mod fat_ro_impl;

pub mod fs {
    pub use crate::fat_ro_impl as fat_ro;
}

pub mod drivers {
    pub use crate::netdev_impl as net;
    pub use crate::queue_impl as queue;
    pub mod virtio {
        pub use crate::virtqueue_impl as virtqueue;
    }
    pub mod bus {
        pub use crate::iommu_regs_impl as iommu_regs;
        pub use crate::mhi_impl as mhi;
        pub use crate::qrtr_impl as qrtr;
        pub use crate::msi_math_impl as msi_math;
    }
    pub mod wifi {
        pub use crate::ath11k_hw_impl as ath11k_hw;
        pub use crate::fw_manifest_impl as fw_manifest;
        pub use crate::qmi_impl as qmi;
        pub use crate::board2_impl as board2;
    }
}

pub mod util {
    pub use crate::csum_impl as csum;
    pub use crate::sha256_impl as sha256;
}

pub use crate::net_impl as net;

/// POPCORN.LOG layout + FAT32 lookup (`klog/fatlog.rs`; the rest of klog is kernel I/O).
#[allow(dead_code)]
#[path = "../../rust/popcorn_kernel/src/klog/fatlog.rs"]
pub mod fatlog_impl;

pub mod klog {
    pub use crate::fatlog_impl as fatlog;
}

#[cfg(test)]
mod csum_tests;
#[cfg(test)]
mod klog_tests;
#[cfg(test)]
mod msi_wifi_tests;
#[cfg(test)]
mod w1_tests;
#[cfg(test)]
mod w2_tests;
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
