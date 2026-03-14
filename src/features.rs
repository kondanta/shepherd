use arc_swap::ArcSwap;
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct RuntimeFlags {
    pub deployments_paused: bool,
    pub dry_run: bool,
}

pub type SharedFlags = Arc<ArcSwap<RuntimeFlags>>;

pub fn new_flags() -> SharedFlags {
    Arc::new(ArcSwap::from_pointee(RuntimeFlags::default()))
}
