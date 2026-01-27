use std::sync::Arc;

use arc_swap::ArcSwapOption;

lazy_static::lazy_static! {
    static ref FEATURES: ArcSwapOption<Vec<&'static str>> = ArcSwapOption::from(None);
}

pub fn set_features(features: Vec<&'static str>) {
    FEATURES.store(Some(Arc::new(features)));
}
