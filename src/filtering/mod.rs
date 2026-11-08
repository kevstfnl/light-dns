mod download;
mod index;
mod parser;
mod rewrite;
mod updater;

#[cfg(test)]
#[path = "../../tests/filtering/trust.rs"]
mod trust;

#[cfg(all(test, target_os = "linux"))]
#[path = "../../tests/performance/filter_memory.rs"]
mod memory;

use crate::config::FilteringConfig;
use arc_swap::ArcSwap;
pub(crate) use index::{Decision, FilterIndex};
use std::{io, sync::Arc};
pub(crate) use updater::Updater;

pub(crate) type SharedFilters = Arc<ArcSwap<FilterIndex>>;

pub(crate) async fn prepare(
    config: FilteringConfig,
) -> io::Result<(SharedFilters, Option<Updater>)> {
    let snapshot = Arc::new(ArcSwap::from_pointee(FilterIndex::default()));
    if !config.enabled {
        return Ok((snapshot, None));
    }
    let updater = Updater::prepare(config, snapshot.clone()).await?;
    Ok((snapshot, Some(updater)))
}
