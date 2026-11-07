mod download;
mod index;
mod parser;
mod updater;

use crate::config::FilteringConfig;
use arc_swap::ArcSwap;
use index::FilterIndex;
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
