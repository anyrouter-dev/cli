//! The model picker's rows and columns: one `ModelRow` per catalog id, then
//! one `PickTable` the TUI lays out by terminal width.

use crate::cmd::models::PickerCatalog;
use crate::http::ModelPricing;
use crate::spawn::{catalog_model_id, is_auto_model};
use crate::tui::table::{Align, Column, PickTable, Styled};
use crate::tui::Tone;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tag {
    /// The catalog bills the model as `free`.
    Free,
    /// Joined the catalog within `NEW_MODEL_DAYS`.
    New,
    /// Priced below its list price.
    Sale,
}

/// Same window as the web catalog's "New" badge.
const NEW_MODEL_DAYS: i64 = 14;

/// One picker row, built once from the catalog. `None` means the catalog has
/// no value, and the row shows a dash.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ModelRow {
    pub(crate) id: String,
    pub(crate) name: Option<String>,
    pub(crate) pricing: Option<ModelPricing>,
    pub(crate) smart: Option<f64>,
    pub(crate) context: Option<i64>,
    pub(crate) tags: Vec<Tag>,
}

pub(crate) fn model_rows(ids: &[String], catalog: &PickerCatalog, now: i64) -> Vec<ModelRow> {
    ids.iter()
        .map(|id| {
            let m = catalog.models.iter().find(|m| {
                let c = catalog_model_id(&m.id);
                &c == id || (is_auto_model(id) && is_auto_model(&c))
            });
            let pricing = m.and_then(|m| m.pricing.clone());
            let created = m.and_then(|m| m.created);
            let mut tags = Vec::new();
            if pricing.as_ref().and_then(|p| p.billing.as_deref()) == Some("free") {
                tags.push(Tag::Free);
            }
            if created.is_some_and(|c| (0..=NEW_MODEL_DAYS * 86_400).contains(&(now - c))) {
                tags.push(Tag::New);
            }
            if pricing.as_ref().is_some_and(|p| {
                p.list_input.is_some_and(|l| l > p.input)
                    || p.list_output.is_some_and(|l| l > p.output)
            }) {
                tags.push(Tag::Sale);
            }
            ModelRow {
                id: id.clone(),
                name: m.and_then(|m| m.name.clone()).filter(|n| n != id),
                pricing,
                smart: catalog.smart.get(id).copied(),
                context: m.and_then(|m| m.context_length).filter(|n| *n > 0),
                tags,
            }
        })
        .collect()
}

fn fmt_usd(v: f64) -> String {
    let digits = if v >= 1.0 { 2 } else { 3 };
    let s = format!("{v:.digits$}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    format!("${s}")
}

fn fmt_context(n: i64) -> String {
    if n >= 1_000_000 {
        let m = format!("{:.1}", n as f64 / 1_000_000.0);
        format!("{}M", m.trim_end_matches(".0"))
    } else if n >= 1_000 {
        format!("{}K", (n as f64 / 1_000.0).round())
    } else {
        n.to_string()
    }
}

fn fmt_tokens(n: i64) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    format!("{out} tokens")
}

const GAUGE: usize = 24;
pub(crate) const KEYS: &str = "type to search  ·  ↑↓ select  ·  ↵ confirm  ·  esc cancel";

fn text(t: impl Into<String>, tone: Tone) -> Styled {
    vec![(t.into(), tone)]
}

fn tag(t: Tag) -> (&'static str, Tone) {
    match t {
        Tag::Free => ("free", Tone::Good),
        Tag::New => ("new", Tone::Model),
        Tag::Sale => ("sale", Tone::Warn),
    }
}

/// 3:1 input to output, the blend Artificial Analysis prices with.
fn blended(row: &ModelRow) -> Option<f64> {
    row.pricing
        .as_ref()
        .map(|p| (3.0 * p.input + p.output) / 4.0)
}

/// Where a price sits among the priced rows, 0 cheapest to 1 priciest.
fn cost_rank(row: &ModelRow, rows: &[ModelRow]) -> Option<f64> {
    let me = blended(row)?;
    let priced: Vec<f64> = rows.iter().filter_map(blended).collect();
    let below = priced.iter().filter(|p| **p < me).count();
    Some(below as f64 / (priced.len().max(2) - 1) as f64)
}

fn gauge(rank: f64) -> Styled {
    let at = ((rank * (GAUGE - 1) as f64).round() as usize).min(GAUGE - 1);
    vec![
        ("cost  ".into(), Tone::Muted),
        ("low ".into(), Tone::Muted),
        ("━".repeat(at), Tone::Good),
        ("●".into(), Tone::Normal),
        ("─".repeat(GAUGE - 1 - at), Tone::Muted),
        (" high".into(), Tone::Muted),
    ]
}

fn detail(row: &ModelRow, rows: &[ModelRow]) -> Vec<Styled> {
    let mut head = text(row.id.clone(), Tone::Model);
    if let Some(name) = &row.name {
        head.push((format!("  {name}"), Tone::Muted));
    }
    let value = |v: Option<String>| match v {
        Some(v) => (v, Tone::Normal),
        None => ("—".into(), Tone::Muted),
    };
    let per_m = |v: Option<f64>| value(v.map(|v| format!("{} / 1M", fmt_usd(v))));
    let p = row.pricing.as_ref();
    let mut prices = Styled::new();
    for (label, v) in [
        ("input ", per_m(p.map(|p| p.input))),
        ("   cached input ", per_m(p.and_then(|p| p.cached_input))),
        ("   output ", per_m(p.map(|p| p.output))),
        ("   context ", value(row.context.map(fmt_tokens))),
    ] {
        prices.push((label.into(), Tone::Muted));
        prices.push(v);
    }
    let mut out = vec![head, prices];
    if let Some(rank) = cost_rank(row, rows) {
        out.push(gauge(rank));
    }
    out
}

pub(crate) fn model_table(rows: &[ModelRow]) -> PickTable {
    let col = |title, align, drop_rank| Column {
        title,
        align,
        drop_rank,
    };
    let usd = |v: Option<f64>| v.map(|v| text(fmt_usd(v), Tone::Normal));
    let cells = rows
        .iter()
        .map(|r| {
            let p = r.pricing.as_ref();
            let mut tags = Styled::new();
            for (i, t) in r.tags.iter().enumerate() {
                let (word, tone) = tag(*t);
                tags.push((
                    if i > 0 {
                        format!(" {word}")
                    } else {
                        word.into()
                    },
                    tone,
                ));
            }
            vec![
                Some(text(r.id.clone(), Tone::Normal)),
                usd(p.map(|p| p.input)),
                usd(p.map(|p| p.output)),
                r.smart.map(|s| text(format!("{s:.1}"), Tone::Normal)),
                r.context.map(|n| text(fmt_context(n), Tone::Normal)),
                Some(tags),
            ]
        })
        .collect();
    PickTable {
        columns: vec![
            col("model", Align::Left, 0),
            col("input", Align::Right, 2),
            col("output", Align::Right, 2),
            col("smart", Align::Right, 4),
            col("context", Align::Right, 3),
            col("", Align::Left, 5),
        ],
        cells,
        details: rows.iter().map(|r| detail(r, rows)).collect(),
        legend: vec![
            (
                "$ per 1M tokens  ·  smart AA index  ·  ".into(),
                Tone::Muted,
            ),
            ("free".into(), Tone::Good),
            (" $0  ".into(), Tone::Muted),
            ("new".into(), Tone::Model),
            (" 14 days  ".into(), Tone::Muted),
            ("sale".into(), Tone::Warn),
            (" below list".into(), Tone::Muted),
        ],
        keys: KEYS,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::cmd::models::{pick_ids, render_model_picker_dump};
    use crate::http::CatalogModel;

    fn cm(id: &str, context_length: Option<i64>) -> CatalogModel {
        CatalogModel {
            id: id.into(),
            context_length,
            ..Default::default()
        }
    }

    fn catalog(models: Vec<CatalogModel>) -> PickerCatalog {
        PickerCatalog {
            models,
            smart: BTreeMap::new(),
        }
    }

    fn priced(id: &str, input: f64, output: f64, billing: &str) -> CatalogModel {
        CatalogModel {
            pricing: Some(ModelPricing {
                input,
                output,
                billing: Some(billing.into()),
                ..Default::default()
            }),
            ..cm(id, Some(200_000))
        }
    }

    const NOW: i64 = 1_800_000_000;
    const DAY: i64 = 86_400;

    #[test]
    fn auto_row_shows_preset_id_not_most_used() {
        let c = catalog(vec![cm("stealth/ox-alpha", Some(1_000_000))]);
        let rows = model_rows(&pick_ids(&c.models), &c, NOW);
        assert_eq!(rows[0].id, "anyrouter/auto");
        assert_eq!(rows[1].id, "stealth/ox-alpha");
        assert_eq!(rows[1].context, Some(1_000_000));
    }

    #[test]
    fn rows_carry_only_catalog_values_and_dash_the_rest() {
        // WHY: the owner's rule is never invent a number; a model the
        // benchmark feed misses must show no smart score, not a zero.
        let mut c = catalog(vec![
            CatalogModel {
                created: Some(NOW - 3 * DAY),
                ..priced("a/new-free", 0.0, 0.0, "free")
            },
            CatalogModel {
                created: Some(NOW - 30 * DAY),
                context_length: Some(0),
                ..priced("b/old-paid", 4.0, 20.0, "byok")
            },
            cm("c/unpriced", None),
        ]);
        c.smart.insert("b/old-paid".into(), 59.9);
        let rows = model_rows(&pick_ids(&c.models), &c, NOW);
        let row = |id: &str| rows.iter().find(|r| r.id == id).unwrap();
        assert_eq!(row("a/new-free").tags, vec![Tag::Free, Tag::New]);
        assert_eq!(row("a/new-free").smart, None);
        assert_eq!(row("b/old-paid").tags, vec![]);
        assert_eq!(row("b/old-paid").smart, Some(59.9));
        assert_eq!(row("b/old-paid").context, None, "0 means unknown");
        assert_eq!(row("c/unpriced").pricing, None);
    }

    #[test]
    fn sale_tag_needs_a_list_price_above_the_price() {
        let mut m = priced("a/x", 1.0, 2.0, "credits");
        let c = catalog(vec![m.clone()]);
        assert!(!model_rows(&pick_ids(&c.models), &c, NOW)[1]
            .tags
            .contains(&Tag::Sale));
        m.pricing.as_mut().unwrap().list_output = Some(4.0);
        let c = catalog(vec![m]);
        assert!(model_rows(&pick_ids(&c.models), &c, NOW)[1]
            .tags
            .contains(&Tag::Sale));
    }

    #[test]
    fn prices_and_context_read_short() {
        assert_eq!(fmt_usd(0.0), "$0");
        assert_eq!(fmt_usd(0.075), "$0.075");
        assert_eq!(fmt_usd(4.0), "$4");
        assert_eq!(fmt_usd(1.25), "$1.25");
        assert_eq!(fmt_context(1_000_000), "1M");
        assert_eq!(fmt_context(1_048_576), "1M");
        assert_eq!(fmt_context(2_000_000), "2M");
        assert_eq!(fmt_context(262_144), "262K");
        assert_eq!(fmt_tokens(1_048_576), "1,048,576 tokens");
    }

    #[test]
    fn picker_typeahead_selects_anyrouter_auto() {
        use crate::tui::{drive_picker, Action, Outcome, PickerState};
        let c = catalog(vec![
            cm("stealth/ox-alpha", Some(1_000_000)),
            cm("openai/gpt-5.4-mini", None),
        ]);
        let ids = pick_ids(&c.models);
        assert_eq!(ids[0], "anyrouter/auto");
        let rows = model_rows(&ids, &c, NOW);
        let mut state =
            PickerState::new("Default model", ids.clone(), Some(0)).with_table(model_table(&rows));
        let typed: Vec<Action> = "anyrouter/auto".chars().map(Action::Char).collect();
        let out = drive_picker(&mut state, &typed);
        assert_eq!(out, Outcome::Continue);
        assert_eq!(state.filtered()[0].1, "anyrouter/auto");
        assert_eq!(state.apply(Action::Enter), Outcome::Selected(0));
    }

    #[test]
    fn typeahead_matches_the_id_not_the_price_columns() {
        // WHY: rows now carry prices; typing "1" must not match "$1.25".
        use crate::tui::{drive_picker, Action, PickerState};
        let c = catalog(vec![priced("a/alpha", 1.25, 10.0, "credits")]);
        let ids = pick_ids(&c.models);
        let rows = model_rows(&ids, &c, NOW);
        let mut state = PickerState::new("m", ids, Some(0)).with_table(model_table(&rows));
        drive_picker(
            &mut state,
            &[Action::Char('1'), Action::Char('.'), Action::Char('2')],
        );
        assert!(state.filtered().is_empty(), "{:?}", state.filtered());
    }

    #[test]
    fn picker_dump_leads_with_preset_not_usage_ranking() {
        let c = catalog(vec![
            cm("stealth/ox-alpha", Some(1_000_000)),
            cm("openai/gpt-5.4-mini", None),
        ]);
        let env = BTreeMap::new();
        let frame = render_model_picker_dump(&c, Some("auto"), &env);
        assert!(frame.contains("anyrouter/auto"), "{frame}");
        assert!(!frame.contains("most used"), "{frame}");
        let auto_at = frame.find("anyrouter/auto").expect("preset");
        let usage_at = frame.find("stealth/ox-alpha");
        if let Some(usage_at) = usage_at {
            assert!(
                auto_at < usage_at,
                "preset must appear before catalog dump:\n{frame}"
            );
        }
    }

    fn dump(c: &PickerCatalog, cols: usize) -> String {
        let env = BTreeMap::from([("COLUMNS".to_string(), cols.to_string())]);
        render_model_picker_dump(c, Some("auto"), &env)
    }

    fn wide_catalog() -> PickerCatalog {
        let mut c = catalog(vec![
            CatalogModel {
                created: Some(crate::civil::now_secs()),
                ..priced(
                    "nvidia/nemotron-3-nano-omni-30b-a3b-reasoning",
                    0.0,
                    0.0,
                    "free",
                )
            },
            priced("anthropic/claude-opus-5-5", 4.0, 20.0, "byok"),
            cm("@preset/aidr", Some(1_000_000)),
        ]);
        c.smart.insert("anthropic/claude-opus-5-5".into(), 59.9);
        c
    }

    fn row_line<'a>(frame: &'a str, id: &str) -> &'a str {
        frame
            .lines()
            .find(|l| l.get(2..).is_some_and(|r| r.starts_with(id)))
            .unwrap_or_else(|| panic!("no row {id}:\n{frame}"))
    }

    #[test]
    fn dump_aligns_columns_at_80_and_120() {
        for cols in [80, 120] {
            let frame = dump(&wide_catalog(), cols);
            let header = frame
                .lines()
                .find(|l| l.contains("output"))
                .expect("header");
            let opus = row_line(&frame, "anthropic/claude-opus-5-5");
            let preset = row_line(&frame, "@preset/aidr");
            let end = |line: &str, word: &str| {
                let w = crate::term::text_width;
                line.find(word).map(|i| w(&line[..i]) + w(word))
            };
            assert_eq!(end(header, "output"), end(opus, "$20"), "{cols}:\n{frame}");
            assert_eq!(end(header, "smart"), end(opus, "59.9"), "{cols}:\n{frame}");
            assert_eq!(
                end(header, "context"),
                end(opus, "200K"),
                "{cols}:\n{frame}"
            );
            assert_eq!(
                end(header, "context"),
                end(preset, "1M"),
                "{cols}:\n{frame}"
            );
            for line in frame.lines() {
                assert!(crate::term::text_width(line) <= cols, "{cols}: {line:?}");
            }
        }
    }

    #[test]
    fn dump_shows_a_dash_where_the_catalog_has_no_value() {
        let frame = dump(&wide_catalog(), 120);
        let preset = row_line(&frame, "@preset/aidr");
        assert_eq!(
            preset.matches('—').count(),
            3,
            "input, output, smart: {preset}"
        );
        assert!(!preset.contains("$0"), "no invented price: {preset}");
        let free = row_line(&frame, "nvidia/nemotron");
        assert!(free.contains("free new"), "{free}");
        assert!(free.contains('—'), "no smart score: {free}");
    }

    #[test]
    fn narrow_terminals_drop_low_priority_columns() {
        let frame = dump(&wide_catalog(), 120);
        let header = frame.lines().find(|l| l.contains("output")).unwrap();
        assert!(
            header.contains("smart") && header.contains("context"),
            "{frame}"
        );
        assert!(
            row_line(&frame, "nvidia/nemotron").contains("free"),
            "{frame}"
        );
        let frame = dump(&wide_catalog(), 60);
        assert!(
            !row_line(&frame, "nvidia/nemotron").contains("free"),
            "tags drop first:\n{frame}"
        );
        let frame = dump(&wide_catalog(), 50);
        let header = frame.lines().find(|l| l.contains("output")).unwrap();
        assert!(!header.contains("smart"), "smart drops next:\n{frame}");
        assert!(header.contains("context"), "{frame}");
        assert!(
            row_line(&frame, "anthropic/claude-opus").contains("$20"),
            "{frame}"
        );
    }

    #[test]
    fn detail_block_names_the_highlighted_model_and_its_prices() {
        let c = wide_catalog();
        let env = BTreeMap::from([("COLUMNS".to_string(), "120".to_string())]);
        let frame = render_model_picker_dump(&c, Some("anthropic/claude-opus-5-5"), &env);
        assert!(frame.contains("input $4 / 1M"), "{frame}");
        assert!(frame.contains("cached input —"), "{frame}");
        assert!(frame.contains("output $20 / 1M"), "{frame}");
        assert!(frame.contains("context 200,000 tokens"), "{frame}");
        assert!(frame.contains("cost  low ━"), "{frame}");
        assert!(frame.contains(KEYS), "{frame}");
    }
}
