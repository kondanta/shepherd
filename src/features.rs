use arc_swap::ArcSwap;
use std::sync::Arc;

/// Runtime feature flags. All flags reset to their default (off) on restart.
/// If shepherd is stopped while `deployments_paused` is true, it will come
/// back with deployments enabled — keep this in mind during incident response.
#[derive(Debug, Clone, Default)]
pub struct RuntimeFlags {
    pub deployments_paused: bool,
    pub dry_run: bool,
}

pub type SharedFlags = Arc<ArcSwap<RuntimeFlags>>;

pub fn new_flags() -> SharedFlags {
    Arc::new(ArcSwap::from_pointee(RuntimeFlags::default()))
}
