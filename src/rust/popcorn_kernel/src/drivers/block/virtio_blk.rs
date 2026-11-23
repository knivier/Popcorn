//! virtio-blk now lives on the driver framework (`drivers/virtio/blk.rs`, M4):
//! grants, the shared transports, `SplitQueue`, MSI-X. This module only keeps the
//! old path working for the block layer.
pub use crate::drivers::virtio::blk::{
    capacity, flush, probe_all, read, read_many, write, write_many, MAX_VIRTIO_DISKS,
};
