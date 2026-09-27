//! Candidate validation before a catalog generation is published.
//!
//! An import builds a *candidate* catalog for one source inside a single
//! transaction: it clears the source's rows, inserts the new ones, then
//! compiles and resolves canonical entities. The previous catalog stays
//! visible to readers until commit. This module decides, from bounded counts
//! alone, whether that candidate may replace the active one. Anything else is
//! rolled back, so the prior active catalog stays exactly as it was.
//!
//! Two independent checks:
//!
//! - **Destructive collapse** ([`check_collapse`]): a refresh that would shrink a
//!   substantial content kind to (almost) nothing is rejected. An endpoint that
//!   answers `[]` or a truncated list without an error is not a reason to delete
//!   a library.
//! - **Integrity** ([`check_integrity`]): what was persisted must be what was
//!   received, every movie row must be bound to a canonical variant, and no
//!   variant may point at a merged (tombstoned) entity.

use std::fmt;

/// Content counts for one source. `series` counts distinct series, not rows:
/// lazily fetched episodes add rows without changing the catalog's breadth.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct KindCounts {
    pub live: u64,
    pub movie: u64,
    pub series: u64,
}

impl KindCounts {
    pub fn total(&self) -> u64 {
        self.live + self.movie + self.series
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKindLabel {
    Live,
    Movie,
    Series,
}

impl ContentKindLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Live => "live channels",
            Self::Movie => "movies",
            Self::Series => "series",
        }
    }
}

/// A kind with fewer items than this is too small for a ratio to mean
/// anything, so its shrinkage is never treated as a collapse.
pub const COLLAPSE_MIN_BASELINE: u64 = 50;

/// A candidate keeping fewer than 1 in [`COLLAPSE_RETAINED_DIVISOR`] of a
/// kind's previous items is treated as a collapse (below 10%).
pub const COLLAPSE_RETAINED_DIVISOR: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntegrityViolation {
    /// Persisted rows of a kind differ from the entries received for it.
    RowCountMismatch {
        kind: ContentKindLabel,
        expected: u64,
        persisted: u64,
    },
    /// Movie rows with no canonical variant: they would be unbrowsable.
    UnboundMovieRows(u64),
    /// Variants owned by a merged entity: they would serve a dead identity.
    VariantsOnMergedEntities(u64),
    /// Merged entities without a redirect to an active survivor.
    DanglingMergedEntities(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishRejection {
    Collapse {
        kind: ContentKindLabel,
        previous: u64,
        candidate: u64,
    },
    Integrity(IntegrityViolation),
}

impl PublishRejection {
    /// Stable, secret-free code for persistence and diagnostics.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Collapse { .. } => "collapse",
            Self::Integrity(IntegrityViolation::RowCountMismatch { .. }) => "row_count_mismatch",
            Self::Integrity(IntegrityViolation::UnboundMovieRows(_)) => "unbound_movie_rows",
            Self::Integrity(IntegrityViolation::VariantsOnMergedEntities(_)) => {
                "variants_on_merged_entities"
            }
            Self::Integrity(IntegrityViolation::DanglingMergedEntities(_)) => {
                "dangling_merged_entities"
            }
        }
    }
}

impl fmt::Display for PublishRejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Collapse {
                kind,
                previous,
                candidate,
            } => write!(
                f,
                "provider returned {candidate} {} instead of about {previous}",
                kind.as_str()
            ),
            Self::Integrity(IntegrityViolation::RowCountMismatch {
                kind,
                expected,
                persisted,
            }) => write!(
                f,
                "stored {persisted} {} but received {expected}",
                kind.as_str()
            ),
            Self::Integrity(IntegrityViolation::UnboundMovieRows(n)) => {
                write!(f, "{n} movies were not bound to the catalog")
            }
            Self::Integrity(IntegrityViolation::VariantsOnMergedEntities(n)) => {
                write!(f, "{n} streams point at merged titles")
            }
            Self::Integrity(IntegrityViolation::DanglingMergedEntities(n)) => {
                write!(f, "{n} merged titles have no surviving title")
            }
        }
    }
}

impl std::error::Error for PublishRejection {}

/// Reject a replacement that would collapse a substantial content kind.
/// Growth, small libraries and proportionate shrinkage all pass.
pub fn check_collapse(previous: KindCounts, candidate: KindCounts) -> Result<(), PublishRejection> {
    for (kind, before, after) in [
        (ContentKindLabel::Live, previous.live, candidate.live),
        (ContentKindLabel::Movie, previous.movie, candidate.movie),
        (ContentKindLabel::Series, previous.series, candidate.series),
    ] {
        if before >= COLLAPSE_MIN_BASELINE
            && after.saturating_mul(COLLAPSE_RETAINED_DIVISOR) < before
        {
            return Err(PublishRejection::Collapse {
                kind,
                previous: before,
                candidate: after,
            });
        }
    }
    Ok(())
}

/// What the candidate transaction actually contains, measured after
/// compilation and resolution.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CandidateReport {
    /// Row counts (series = rows, not distinct titles).
    pub rows: KindCounts,
    pub unbound_movie_rows: u64,
    pub variants_on_merged_entities: u64,
    pub dangling_merged_entities: u64,
}

/// Verify a compiled candidate against the entries received for it.
/// `received` counts rows per kind, as inserted by the importer.
pub fn check_integrity(
    received: KindCounts,
    report: &CandidateReport,
) -> Result<(), PublishRejection> {
    for (kind, expected, persisted) in [
        (ContentKindLabel::Live, received.live, report.rows.live),
        (ContentKindLabel::Movie, received.movie, report.rows.movie),
        (
            ContentKindLabel::Series,
            received.series,
            report.rows.series,
        ),
    ] {
        if expected != persisted {
            return Err(PublishRejection::Integrity(
                IntegrityViolation::RowCountMismatch {
                    kind,
                    expected,
                    persisted,
                },
            ));
        }
    }
    let checks = [
        (
            report.unbound_movie_rows,
            IntegrityViolation::UnboundMovieRows(report.unbound_movie_rows),
        ),
        (
            report.variants_on_merged_entities,
            IntegrityViolation::VariantsOnMergedEntities(report.variants_on_merged_entities),
        ),
        (
            report.dangling_merged_entities,
            IntegrityViolation::DanglingMergedEntities(report.dangling_merged_entities),
        ),
    ];
    match checks.into_iter().find(|(n, _)| *n > 0) {
        Some((_, violation)) => Err(PublishRejection::Integrity(violation)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(live: u64, movie: u64, series: u64) -> KindCounts {
        KindCounts {
            live,
            movie,
            series,
        }
    }

    #[test]
    fn an_empty_endpoint_cannot_wipe_a_substantial_kind() {
        let err =
            check_collapse(counts(9_000, 28_000, 5_000), counts(9_000, 0, 5_000)).unwrap_err();
        assert_eq!(
            err,
            PublishRejection::Collapse {
                kind: ContentKindLabel::Movie,
                previous: 28_000,
                candidate: 0
            }
        );
        assert_eq!(err.code(), "collapse");
        assert!(err.to_string().contains("0 movies"));
    }

    #[test]
    fn a_truncated_list_below_ten_percent_is_a_collapse() {
        assert!(check_collapse(counts(1_000, 0, 0), counts(99, 0, 0)).is_err());
        assert!(check_collapse(counts(1_000, 0, 0), counts(100, 0, 0)).is_ok());
    }

    #[test]
    fn growth_small_libraries_and_first_imports_always_pass() {
        assert!(check_collapse(counts(0, 0, 0), counts(10, 20, 30)).is_ok());
        assert!(check_collapse(counts(100, 100, 100), counts(5_000, 5_000, 5_000)).is_ok());
        // Under the baseline a ratio is meaningless: 49 → 0 is allowed.
        assert!(check_collapse(counts(49, 49, 49), counts(0, 0, 0)).is_ok());
        assert!(check_collapse(counts(50, 0, 0), counts(0, 0, 0)).is_err());
    }

    #[test]
    fn integrity_requires_every_received_row_and_binding() {
        let received = counts(3, 2, 1);
        let ok = CandidateReport {
            rows: counts(3, 2, 1),
            ..CandidateReport::default()
        };
        assert!(check_integrity(received, &ok).is_ok());

        let lost = CandidateReport {
            rows: counts(3, 1, 1),
            ..ok
        };
        assert_eq!(
            check_integrity(received, &lost).unwrap_err().code(),
            "row_count_mismatch"
        );

        for (report, code) in [
            (
                CandidateReport {
                    unbound_movie_rows: 1,
                    ..ok
                },
                "unbound_movie_rows",
            ),
            (
                CandidateReport {
                    variants_on_merged_entities: 2,
                    ..ok
                },
                "variants_on_merged_entities",
            ),
            (
                CandidateReport {
                    dangling_merged_entities: 1,
                    ..ok
                },
                "dangling_merged_entities",
            ),
        ] {
            assert_eq!(check_integrity(received, &report).unwrap_err().code(), code);
        }
    }
}
