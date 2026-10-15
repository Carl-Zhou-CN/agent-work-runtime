use crate::{Locator, SourceSnapshot};
use awr_core::{Freshness, Result, Source};
use awr_store::Store;
use std::path::Path;

#[derive(Debug)]
pub struct SourceObservation {
    pub source: Source,
    pub changed: bool,
    pub snapshot: Option<SourceSnapshot>,
    pub error: Option<awr_core::Error>,
}

/// Re-read authority. Equality alone cannot promote an unindexed or failed source to fresh.
pub fn observe_source(
    store: &mut Store,
    source: &Source,
    root: &Path,
    locator: &Locator,
    cap: u64,
) -> Result<SourceObservation> {
    match locator.read(root, cap) {
        Ok(snapshot) => {
            let changed = source.fingerprint != snapshot.fingerprint;
            // Only an actual byte change demotes the source. Re-marking an
            // unchanged source stale would emit a freshness event and advance
            // the project revision even though no content moved, which forces
            // concurrent writers to refresh their expected revision for
            // nothing. A non-fresh source that rereads identical bytes keeps
            // its state here and is handled by the unchanged fast path (or a
            // single reprojection) in the indexer.
            let source = if changed {
                store.mark_source_freshness(source, Freshness::Stale)?
            } else {
                let current = store.source(source.project_id, source.id)?;
                if current.revision != source.revision
                    || current.fingerprint != snapshot.fingerprint
                    || current.config != source.config
                {
                    return Err(awr_core::Error::SourceConflict(
                        "source changed during observation".into(),
                    ));
                }
                current
            };
            Ok(SourceObservation {
                source,
                changed,
                snapshot: Some(snapshot),
                error: None,
            })
        }
        Err(error) => Ok(SourceObservation {
            source: store.mark_source_freshness(source, Freshness::Unavailable)?,
            changed: false,
            snapshot: None,
            error: Some(error),
        }),
    }
}
