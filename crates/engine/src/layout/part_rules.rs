//! The part-kind legality rules — the structural grammar of a layout's bands
//! (one body, at most one header/footer, grand summaries once per side of the
//! body, subsummaries free) and where each kind inserts. Self-contained pure
//! functions over the already-loaded `&[PartMeta]` — no DB access — so the
//! rules are separately testable from the `meta_part` CRUD in
//! [`parts`](super::parts) that enforces them.

use anyhow::{Result, bail};

use super::{PartKind, PartMeta};

pub(super) fn validate_part_create(parts: &[PartMeta], kind: PartKind) -> Result<()> {
    match kind {
        PartKind::Header | PartKind::Body | PartKind::Footer => {
            if parts.iter().any(|p| p.kind == kind) {
                bail!("layout already has a {} part", kind.as_str());
            }
        }
        PartKind::GrandSummary => {
            if has_trailing_grand_summary(parts) && has_leading_grand_summary(parts) {
                bail!("layout already has leading and trailing grand summary parts");
            }
        }
        PartKind::SubSummary => {}
    }
    Ok(())
}

pub(super) fn validate_part_kind_change(
    parts: &[PartMeta],
    part_id: i64,
    kind: PartKind,
) -> Result<()> {
    match kind {
        PartKind::Header | PartKind::Body | PartKind::Footer => {
            if parts.iter().any(|p| p.id != part_id && p.kind == kind) {
                bail!("layout already has a {} part", kind.as_str());
            }
        }
        PartKind::GrandSummary => {
            let Some(part) = parts.iter().find(|p| p.id == part_id) else {
                return Ok(());
            };
            let body_pos = parts
                .iter()
                .find(|p| p.kind == PartKind::Body)
                .map(|p| p.position)
                .unwrap_or(part.position);
            let wants_trailing = part.position > body_pos;
            let duplicate = parts.iter().any(|p| {
                p.id != part_id
                    && p.kind == PartKind::GrandSummary
                    && ((p.position > body_pos) == wants_trailing)
            });
            if duplicate {
                bail!("layout already has a grand summary on that side of the body");
            }
        }
        PartKind::SubSummary => {}
    }
    Ok(())
}

pub(super) fn insertion_position(parts: &[PartMeta], kind: PartKind) -> i64 {
    let len = parts.len() as i64;
    let body_pos = parts
        .iter()
        .find(|p| p.kind == PartKind::Body)
        .map(|p| p.position);
    let footer_pos = parts
        .iter()
        .find(|p| p.kind == PartKind::Footer)
        .map(|p| p.position);
    match kind {
        PartKind::Header => 0,
        PartKind::Body => footer_pos.unwrap_or(len),
        PartKind::Footer => len,
        PartKind::SubSummary => parts
            .iter()
            .filter(|p| {
                p.kind == PartKind::Footer
                    || (p.kind == PartKind::GrandSummary
                        && body_pos.is_some_and(|body| p.position > body))
            })
            .map(|p| p.position)
            .min()
            .unwrap_or(len),
        PartKind::GrandSummary => {
            if !has_trailing_grand_summary(parts) {
                footer_pos.unwrap_or(len)
            } else {
                body_pos.unwrap_or(len).max(0)
            }
        }
    }
}

fn has_leading_grand_summary(parts: &[PartMeta]) -> bool {
    let Some(body_pos) = parts
        .iter()
        .find(|p| p.kind == PartKind::Body)
        .map(|p| p.position)
    else {
        return parts.iter().any(|p| p.kind == PartKind::GrandSummary);
    };
    parts
        .iter()
        .any(|p| p.kind == PartKind::GrandSummary && p.position < body_pos)
}

fn has_trailing_grand_summary(parts: &[PartMeta]) -> bool {
    let Some(body_pos) = parts
        .iter()
        .find(|p| p.kind == PartKind::Body)
        .map(|p| p.position)
    else {
        return false;
    };
    parts
        .iter()
        .any(|p| p.kind == PartKind::GrandSummary && p.position > body_pos)
}
