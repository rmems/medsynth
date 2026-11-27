//! Passage retrieval for citation-support audit: cache handling, LibreTexts
//! locator resolution, fetching, and HTML/MathML text extraction.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use ego_tree::NodeRef;
use scraper::node::Node;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

use super::{AccessEntry, sha256_text};

const PASSAGE_CACHE_VERSION: u32 = 4;

#[derive(Debug, Clone)]
pub(crate) struct Passage {
    pub(crate) url: String,
    pub(crate) text: String,
    pub(crate) sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CachedPassage {
    cache_version: u32,
    source_id: String,
    locator: String,
    retrieval: String,
    base_url: String,
    url: String,
    text: String,
    passage_sha256: String,
}

struct CacheIdentity<'a> {
    entry: &'a AccessEntry,
    locator: &'a str,
}

fn cache_path(cache_dir: &Path, identity: &CacheIdentity<'_>) -> PathBuf {
    cache_dir
        .join(&identity.entry.source_id)
        .join(format!("{}.json", sha256_text(identity.locator)))
}

fn cached_passage_matches(
    cached: &CachedPassage,
    identity: &CacheIdentity<'_>,
    expected_url: &str,
) -> bool {
    [
        cached.cache_version == PASSAGE_CACHE_VERSION,
        cached.source_id == identity.entry.source_id,
        cached.locator == identity.locator,
        cached.retrieval == identity.entry.retrieval,
        cached.base_url == identity.entry.base_url,
        cached.url == expected_url,
        cached.passage_sha256 == sha256_text(&cached.text),
    ]
    .into_iter()
    .all(|matches| matches)
}

fn read_cached_passage(
    path: &Path,
    identity: &CacheIdentity<'_>,
    expected_url: &str,
) -> Option<Passage> {
    let text = fs::read_to_string(path).ok()?;
    let cached: CachedPassage = serde_json::from_str(&text).ok()?;
    if !cached_passage_matches(&cached, identity, expected_url) {
        return None;
    }
    Some(Passage {
        url: cached.url,
        text: cached.text,
        sha256: cached.passage_sha256,
    })
}

fn write_cached_passage(
    path: &Path,
    identity: &CacheIdentity<'_>,
    passage: &Passage,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let cached = CachedPassage {
        cache_version: PASSAGE_CACHE_VERSION,
        source_id: identity.entry.source_id.clone(),
        locator: identity.locator.to_owned(),
        retrieval: identity.entry.retrieval.clone(),
        base_url: identity.entry.base_url.clone(),
        url: passage.url.clone(),
        text: passage.text.clone(),
        passage_sha256: passage.sha256.clone(),
    };
    fs::write(path, serde_json::to_vec_pretty(&cached)?)?;
    Ok(())
}

fn fetch_text(url: &str, base_url: &str) -> anyhow::Result<String> {
    let base_url = base_url.to_owned();
    let redirect_policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= 10 {
            attempt.error("too many passage redirects")
        } else if url_stays_under_base(attempt.url().as_str(), &base_url) {
            attempt.follow()
        } else {
            attempt.error("passage redirect escapes the allowlisted source root")
        }
    });
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .redirect(redirect_policy)
        .build()
        .context("cannot build passage-fetch client")?
        .get(url)
        .send()
        .with_context(|| format!("cannot fetch {url}"))?
        .error_for_status()
        .with_context(|| format!("HTTP error fetching {url}"))?
        .text()
        .with_context(|| format!("cannot decode {url} as text"))
}

fn has_dot_segments(suffix: &str) -> bool {
    suffix
        .split('/')
        .any(|segment| segment == "." || segment == "..")
}

fn has_encoded_or_separator_traversal(suffix: &str) -> bool {
    suffix.to_ascii_lowercase().contains("%2e") || suffix.contains('\\')
}

fn url_stays_under_base(url: &str, base_url: &str) -> bool {
    let base = base_url.trim_end_matches('/');
    let prefix = format!("{base}/");
    if !url.starts_with(&prefix) {
        return false;
    }
    let suffix = &url[prefix.len()..];
    !(has_dot_segments(suffix) || has_encoded_or_separator_traversal(suffix))
}

fn resolve_libretexts_url(entry: &AccessEntry, locator: &str) -> anyhow::Result<String> {
    let url = entry.locator_urls.get(locator).ok_or_else(|| {
        anyhow::anyhow!(
            "locator `{locator}` is not allowlisted for `{}`",
            entry.source_id
        )
    })?;
    if !url_stays_under_base(url, &entry.base_url) {
        bail!(
            "allowlisted URL for `{locator}` escapes the `{}` base URL or contains path traversal",
            entry.source_id
        );
    }
    Ok(url.clone())
}

fn element_children<'a>(node: NodeRef<'a, Node>) -> impl Iterator<Item = NodeRef<'a, Node>> {
    node.children().filter(|child| child.value().is_element())
}

fn append_child_mathml(node: NodeRef<'_, Node>, out: &mut String) {
    for child in node.children() {
        append_mathml(child, out);
    }
}

fn append_mathml_parts(node: NodeRef<'_, Node>, out: &mut String, separator: &str) {
    let parts = element_children(node).collect::<Vec<_>>();
    if parts.len() == 2 {
        append_mathml(parts[0], out);
        out.push_str(separator);
        append_mathml(parts[1], out);
    } else {
        append_child_mathml(node, out);
    }
}

fn append_mathml_subsup(node: NodeRef<'_, Node>, out: &mut String) {
    let parts = element_children(node).collect::<Vec<_>>();
    if parts.len() == 3 {
        append_mathml(parts[0], out);
        out.push_str("_{");
        append_mathml(parts[1], out);
        out.push_str("}^{");
        append_mathml(parts[2], out);
        out.push('}');
    } else {
        append_child_mathml(node, out);
    }
}

fn append_mathml_table(node: NodeRef<'_, Node>, out: &mut String) {
    for (index, row) in element_children(node).enumerate() {
        if index > 0 {
            out.push_str(" ; ");
        }
        append_mathml(row, out);
    }
}

fn append_mathml_row(node: NodeRef<'_, Node>, out: &mut String) {
    for cell in element_children(node) {
        append_mathml(cell, out);
        out.push(' ');
    }
}

fn append_mathml(node: NodeRef<'_, Node>, out: &mut String) {
    if let Some(text) = node.value().as_text() {
        out.push_str(&text.text);
        return;
    }
    let Some(element) = node.value().as_element() else {
        return;
    };
    match element.name() {
        "annotation" | "annotation-xml" => {}
        "msqrt" => {
            out.push_str("sqrt(");
            append_child_mathml(node, out);
            out.push(')');
        }
        "mfrac" => {
            out.push('(');
            append_mathml_parts(node, out, ")/(");
            out.push(')');
        }
        "msub" => append_mathml_parts(node, out, "_"),
        "msup" => append_mathml_parts(node, out, "^"),
        "msubsup" => append_mathml_subsup(node, out),
        "mtable" => append_mathml_table(node, out),
        "mtr" => append_mathml_row(node, out),
        "mspace" => out.push(' '),
        _ => append_child_mathml(node, out),
    }
}

fn mathml_text(node: NodeRef<'_, Node>) -> String {
    let mut out = String::new();
    append_mathml(node, &mut out);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn append_html_text(node: NodeRef<'_, Node>, out: &mut String) {
    if let Some(text) = node.value().as_text() {
        out.push_str(&text.text);
        out.push(' ');
        return;
    }
    let Some(element) = node.value().as_element() else {
        return;
    };
    match element.name() {
        "script" | "style" | "noscript" => {}
        "math" => {
            out.push_str(&mathml_text(node));
            out.push(' ');
        }
        _ => {
            for child in node.children() {
                append_html_text(child, out);
            }
        }
    }
}

/// Extract the visible LibreTexts content container, preserving the operators
/// represented structurally by common MathML elements.
pub fn extract_main_text(html: &str) -> String {
    let document = Html::parse_document(html);
    let content = Selector::parse("section.mt-content-container")
        .expect("LibreTexts content selector is valid");
    let fallback = Selector::parse("main").expect("main selector is valid");
    let mut text = String::new();
    let root = document
        .select(&content)
        .next()
        .or_else(|| document.select(&fallback).next());
    if let Some(root) = root {
        for child in root.children() {
            append_html_text(child, &mut text);
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn retrieve_passage(
    entry: &AccessEntry,
    locator: &str,
    cache_dir: &Path,
    offline: bool,
) -> anyhow::Result<Passage> {
    let url = resolve_libretexts_url(entry, locator)?;
    let identity = CacheIdentity { entry, locator };
    let path = cache_path(cache_dir, &identity);
    if let Some(passage) = read_cached_passage(&path, &identity, &url) {
        return Ok(passage);
    }
    if entry.retrieval != "libretexts-web" {
        bail!("unsupported retrieval method `{}`", entry.retrieval);
    }
    if offline {
        bail!("no cached passage and offline mode is enabled");
    }
    let html = fetch_text(&url, &entry.base_url)?;
    let text = extract_main_text(&html);
    if text.is_empty() {
        bail!("no main text extracted from {url}");
    }
    let passage = Passage {
        url,
        sha256: sha256_text(&text),
        text,
    };
    write_cached_passage(&path, &identity, &passage)?;
    Ok(passage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn extracts_visible_libretexts_content_text() {
        let html = r#"<!doctype html><html><body><nav>skip me</nav><section class="mt-content-container"><h1>Section</h1><p>Gap junctions connect cells.</p></section></body></html>"#;
        assert_eq!(
            extract_main_text(html),
            "Section Gap junctions connect cells."
        );
    }

    #[test]
    fn extracts_mathml_fractions_subscripts_and_square_roots() {
        let html = r#"<section class="mt-content-container"><math><semantics><mrow><mo>|</mo><mi>v</mi><mo>|</mo><mo>=</mo><msqrt><mrow><mfrac><mrow><msub><mi>F</mi><mi>T</mi></msub></mrow><mi>μ</mi></mfrac></mrow></msqrt></mrow><annotation-xml encoding="MathML-Content"><mrow><mi>duplicate</mi></mrow></annotation-xml></semantics></math></section>"#;
        assert_eq!(extract_main_text(html), "|v|=sqrt((F_T)/(μ))");
    }

    #[test]
    fn extracts_integral_limits_and_aligned_equation_rows() {
        let html = r#"<section class="mt-content-container"><math><semantics><mtable><mtr><mtd><msubsup><mo>∫</mo><mrow><msub><mi>p</mi><mn>0</mn></msub></mrow><mi>p</mi></msubsup><mrow><mi>d</mi><mi>p</mi></mrow></mtd><mtd><mo>=</mo></mtd><mtd><mtext>−</mtext><msubsup><mo>∫</mo><mn>0</mn><mrow><mtext>−</mtext><mi>h</mi></mrow></msubsup><mi>ρ</mi><mi>g</mi><mi>d</mi><mi>y</mi></mtd></mtr><mtr><mtd><mi>p</mi><mo>−</mo><msub><mi>p</mi><mn>0</mn></msub></mtd><mtd><mo>=</mo></mtd><mtd><mi>ρ</mi><mi>g</mi><mi>h</mi></mtd></mtr><mtr><mtd><mi>p</mi></mtd><mtd><mo>=</mo></mtd><mtd><msub><mi>p</mi><mn>0</mn></msub><mo>+</mo><mi>ρ</mi><mi>g</mi><mi>h</mi><mo>.</mo></mtd></mtr></mtable><annotation-xml encoding="MathML-Content"><mi>duplicate</mi></annotation-xml></semantics></math></section>"#;
        assert_eq!(
            extract_main_text(html),
            "∫_{p_0}^{p}dp = −∫_{0}^{−h}ρgdy ; p−p_0 = ρgh ; p = p_0+ρgh."
        );
    }

    fn access_entry() -> AccessEntry {
        let base_url = "https://med.libretexts.org/book".to_owned();
        let locator_urls = BTreeMap::from([(
            "sec4.2A:characteristics-of-epithelial-tissue".to_owned(),
            format!("{base_url}/4.2A%3A_Characteristics_of_Epithelial_Tissue"),
        )]);
        AccessEntry {
            source_id: "source".to_owned(),
            license: "license".to_owned(),
            retrieval: "libretexts-web".to_owned(),
            base_url,
            locator_urls,
        }
    }

    #[test]
    fn locator_resolution_uses_the_rights_allowlist() {
        let entry = access_entry();
        assert_eq!(
            resolve_libretexts_url(&entry, "sec4.2A:characteristics-of-epithelial-tissue")
                .expect("allowlisted locator"),
            "https://med.libretexts.org/book/4.2A%3A_Characteristics_of_Epithelial_Tissue"
        );
        assert!(resolve_libretexts_url(&entry, "sec9.9:missing").is_err());
    }

    #[test]
    fn locator_resolution_rejects_base_escape_and_traversal() {
        let mut entry = access_entry();
        let locator = "sec4.2A:characteristics-of-epithelial-tissue";
        entry.locator_urls.insert(
            locator.to_owned(),
            "https://med.libretexts.org/book-evil/page".to_owned(),
        );
        assert!(resolve_libretexts_url(&entry, locator).is_err());

        for suffix in ["../other/page", "a/%2e%2e/other", "a\\..\\other"] {
            entry.locator_urls.insert(
                locator.to_owned(),
                format!("https://med.libretexts.org/book/{suffix}"),
            );
            assert!(resolve_libretexts_url(&entry, locator).is_err(), "{suffix}");
        }
    }

    #[test]
    fn offline_retrieval_without_valid_cache_fails_closed() {
        let entry = access_entry();
        let dir = std::env::temp_dir().join(format!(
            "medsynth-passage-cache-test-{}",
            std::process::id()
        ));
        let error = retrieve_passage(
            &entry,
            "sec4.2A:characteristics-of-epithelial-tissue",
            &dir,
            true,
        )
        .expect_err("offline retrieval cannot invent a passage");
        assert!(error.to_string().contains("offline mode"));
    }

    #[test]
    fn stale_cache_for_a_different_allowlisted_url_is_rejected() {
        let entry = access_entry();
        let locator = "sec4.2A:characteristics-of-epithelial-tissue";
        let dir = std::env::temp_dir().join(format!(
            "medsynth-stale-passage-cache-test-{}",
            std::process::id()
        ));
        let identity = CacheIdentity {
            entry: &entry,
            locator,
        };
        let path = cache_path(&dir, &identity);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let text = "cached passage";
        let mut cached = CachedPassage {
            cache_version: PASSAGE_CACHE_VERSION,
            source_id: entry.source_id.clone(),
            locator: locator.to_owned(),
            retrieval: entry.retrieval.clone(),
            base_url: entry.base_url.clone(),
            url: "https://med.libretexts.org/book/older-locator".to_owned(),
            text: text.to_owned(),
            passage_sha256: sha256_text(text),
        };
        fs::write(&path, serde_json::to_vec_pretty(&cached).unwrap()).unwrap();
        assert!(
            read_cached_passage(&path, &identity, entry.locator_urls.get(locator).unwrap(),)
                .is_none()
        );

        cached.url = entry.locator_urls.get(locator).unwrap().clone();
        fs::write(&path, serde_json::to_vec_pretty(&cached).unwrap()).unwrap();
        assert_eq!(
            read_cached_passage(&path, &identity, entry.locator_urls.get(locator).unwrap())
                .unwrap()
                .text,
            text
        );
    }
}
