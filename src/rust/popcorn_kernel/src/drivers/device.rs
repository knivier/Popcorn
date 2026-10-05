//! Bound device instance in the driver network.

use alloc::string::String;

use super::class::{BlockOps, CharOps, FbOps};
use super::driver::DriverId;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DeviceClass {
    Char,
    Block,
    Framebuffer,
    Info,
}

/// Runtime device node (name shown under `/dev`).
pub struct Device {
    pub name: String,
    pub class: DeviceClass,
    pub driver: DriverId,
    pub ready: bool,
    pub char_ops: Option<&'static CharOps>,
    pub block_ops: Option<&'static BlockOps>,
    pub fb_ops: Option<&'static FbOps>,
}

impl Device {
    pub fn new_char(name: &str, driver: DriverId, ops: &'static CharOps) -> Self {
        Self {
            name: String::from(name),
            class: DeviceClass::Char,
            driver,
            ready: true,
            char_ops: Some(ops),
            block_ops: None,
            fb_ops: None,
        }
    }

    pub fn class_name(&self) -> &'static str {
        match self.class {
            DeviceClass::Char => "char",
            DeviceClass::Block => "block",
            DeviceClass::Framebuffer => "fb",
            DeviceClass::Info => "info",
        }
    }
}
