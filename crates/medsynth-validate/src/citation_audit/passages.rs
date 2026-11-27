//! Passage retrieval for citation-support audit: cache handling, OpenStax
//! locator resolution, fetching, and HTML/MathML text extraction.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use ego_tree::NodeRef;
use scraper::node::Node;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

use super::{AccessEntry, sha256_text};

const PASSAGE_CACHE_VERSION: u32 = 2;

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
    book: String,
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

fn cached_passage_matches(cached: &CachedPassage, identity: &CacheIdentity<'_>) -> bool {
    [
        cached.cache_version == PASSAGE_CACHE_VERSION,
        cached.source_id == identity.entry.source_id,
        cached.locator == identity.locator,
        cached.retrieval == identity.entry.retrieval,
        cached.book == book_for_locator(identity.entry, identity.locator),
        cached.passage_sha256 == sha256_text(&cached.text),
    ]
    .into_iter()
    .all(|matches| matches)
}

fn read_cached_passage(path: &Path, identity: &CacheIdentity<'_>) -> Option<Passage> {
    let text = fs::read_to_string(path).ok()?;
    let cached: CachedPassage = serde_json::from_str(&text).ok()?;
    if !cached_passage_matches(&cached, identity) {
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
        book: book_for_locator(identity.entry, identity.locator),
        url: passage.url.clone(),
        text: passage.text.clone(),
        passage_sha256: passage.sha256.clone(),
    };
    fs::write(path, serde_json::to_vec_pretty(&cached)?)?;
    Ok(())
}

fn fetch_text(url: &str) -> anyhow::Result<String> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
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

pub(crate) struct OpenStaxTarget<'a> {
    book: &'a str,
    chapter: &'a str,
    slug: &'a str,
}

fn page_candidate(file: &str, target: &OpenStaxTarget<'_>) -> bool {
    let Some(rest) = file.strip_prefix(&format!("{}-", target.chapter)) else {
        return false;
    };
    let Some(section) = rest.strip_suffix(&format!("-{}", target.slug)) else {
        return false;
    };
    !section.is_empty() && section.chars().all(|c| c.is_ascii_digit())
}

/// Find `chapter-section-slug` in an OpenStax chapter introduction page.
pub(crate) fn find_openstax_page_path(
    index_html: &str,
    target: &OpenStaxTarget<'_>,
) -> Option<String> {
    let document = Html::parse_document(index_html);
    let selector = Selector::parse("a[href]").ok()?;
    document
        .select(&selector)
        .filter_map(|element| element.value().attr("href"))
        .filter_map(|href| {
            let clean = href.split('#').next()?.trim_end_matches('/');
            let file = clean.rsplit('/').next()?;
            page_candidate(file, target)
                .then(|| format!("https://openstax.org/books/{}/pages/{file}", target.book))
        })
        .next()
}

struct DirectSection<'a> {
    chapter: &'a str,
    section: &'a str,
    slug: &'a str,
}

fn ascii_digits(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit())
}

fn parse_direct_section(locator: &str) -> Option<DirectSection<'_>> {
    let (prefix, slug) = locator.split_once(':')?;
    let section = prefix.strip_prefix("sec")?;
    let (chapter, section) = section.split_once('.')?;
    if !ascii_digits(chapter) || !ascii_digits(section) {
        return None;
    }
    Some(DirectSection {
        chapter,
        section,
        slug,
    })
}

fn direct_section_url(book: &str, section: &DirectSection<'_>) -> String {
    format!(
        "https://openstax.org/books/{book}/pages/{}-{}-{}",
        section.chapter, section.section, section.slug
    )
}

fn chapter_locator(locator: &str) -> Option<(String, String)> {
    let (prefix, slug) = locator.rsplit_once(':')?;
    if let Some(chapter) = prefix.strip_prefix("ch") {
        return Some((chapter.to_owned(), slug.to_owned()));
    }
    let (_, chapter) = prefix.rsplit_once(":ch")?;
    Some((chapter.to_owned(), slug.to_owned()))
}

fn book_for_locator(entry: &AccessEntry, locator: &str) -> String {
    if let Some(volume) = locator
        .strip_prefix("vol")
        .and_then(|rest| rest.split(':').next())
        .filter(|volume| volume.chars().all(|c| c.is_ascii_digit()))
    {
        return entry.book.replace("{volume}", volume);
    }
    entry.book.clone()
}

fn resolve_openstax_url(
    entry: &AccessEntry,
    locator: &str,
    offline: bool,
) -> anyhow::Result<String> {
    let book = book_for_locator(entry, locator);
    if let Some(section) = parse_direct_section(locator) {
        return Ok(direct_section_url(&book, &section));
    }
    let (chapter, slug) = chapter_locator(locator)
        .ok_or_else(|| anyhow::anyhow!("unsupported OpenStax locator `{locator}`"))?;
    if offline {
        bail!("locator `{locator}` requires chapter-index resolution and no cached passage exists");
    }
    let index_url = format!("https://openstax.org/books/{book}/pages/{chapter}-introduction");
    let index = fetch_text(&index_url)?;
    let target = OpenStaxTarget {
        book: &book,
        chapter: &chapter,
        slug: &slug,
    };
    find_openstax_page_path(&index, &target)
        .ok_or_else(|| anyhow::anyhow!("locator `{locator}` not found in {index_url}"))
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

/// Extract the visible text of OpenStax's `<main>` content, preserving the
/// operators represented structurally by common MathML elements.
pub fn extract_main_text(html: &str) -> String {
    let document = Html::parse_document(html);
    let selector = Selector::parse("main").expect("main selector is valid");
    let mut text = String::new();
    if let Some(main) = document.select(&selector).next() {
        for child in main.children() {
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
    let identity = CacheIdentity { entry, locator };
    let path = cache_path(cache_dir, &identity);
    if let Some(passage) = read_cached_passage(&path, &identity) {
        return Ok(passage);
    }
    if entry.retrieval != "openstax-web" {
        bail!("unsupported retrieval method `{}`", entry.retrieval);
    }
    if offline {
        bail!("no cached passage and offline mode is enabled");
    }
    let url = resolve_openstax_url(entry, locator, offline)?;
    let html = fetch_text(&url)?;
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

    fn target<'a>(chapter: &'a str, slug: &'a str) -> OpenStaxTarget<'a> {
        OpenStaxTarget {
            book: "introduction-sociology-3e",
            chapter,
            slug,
        }
    }

    #[test]
    fn extracts_visible_openstax_main_text() {
        let html = r#"<!doctype html><html><body><nav>skip me</nav><main><h1>Section</h1><p>Gap junctions connect cells.</p></main></body></html>"#;
        assert_eq!(
            extract_main_text(html),
            "Section Gap junctions connect cells."
        );
    }

    #[test]
    fn extracts_mathml_fractions_subscripts_and_square_roots() {
        let html = r#"<main><math><semantics><mrow><mo>|</mo><mi>v</mi><mo>|</mo><mo>=</mo><msqrt><mrow><mfrac><mrow><msub><mi>F</mi><mi>T</mi></msub></mrow><mi>μ</mi></mfrac></mrow></msqrt></mrow><annotation-xml encoding="MathML-Content"><mrow><mi>duplicate</mi></mrow></annotation-xml></semantics></math></main>"#;
        assert_eq!(extract_main_text(html), "|v|=sqrt((F_T)/(μ))");
    }

    #[test]
    fn resolves_chapter_slug_from_index_links() {
        let html = r#"<html><body><a href="19-3-health-in-the-united-states">x</a></body></html>"#;
        assert_eq!(
            find_openstax_page_path(html, &target("19", "health-in-the-united-states")),
            Some(
                "https://openstax.org/books/introduction-sociology-3e/pages/19-3-health-in-the-united-states"
                    .to_owned()
            )
        );
    }

    #[test]
    fn rejects_partial_chapter_slug_matches() {
        assert!(!page_candidate(
            "19-30-health-in-the-united-states",
            &target("1", "health-in-the-united-states")
        ));
        assert!(!page_candidate(
            "19-3-health-in-the-united-states-extra",
            &target("19", "health-in-the-united-states")
        ));
    }

    #[test]
    fn direct_section_locator_maps_to_page_url() {
        let section = parse_direct_section("sec4.2:epithelial-tissue").expect("direct section");
        assert_eq!(
            direct_section_url("anatomy-and-physiology-2e", &section),
            "https://openstax.org/books/anatomy-and-physiology-2e/pages/4-2-epithelial-tissue"
        );
    }

    fn access_entry(book: &str) -> AccessEntry {
        AccessEntry {
            source_id: "source".to_owned(),
            license: "license".to_owned(),
            retrieval: "openstax-web".to_owned(),
            book: book.to_owned(),
        }
    }

    #[test]
    fn cached_passage_is_bound_to_policy_book_and_hash() {
        let entry = access_entry("new-book");
        let identity = CacheIdentity {
            entry: &entry,
            locator: "sec1.1:slug",
        };
        let text = "passage text";
        let cached = CachedPassage {
            cache_version: PASSAGE_CACHE_VERSION,
            source_id: "source".to_owned(),
            locator: "sec1.1:slug".to_owned(),
            retrieval: "openstax-web".to_owned(),
            book: "old-book".to_owned(),
            url: "https://example.test/page".to_owned(),
            text: text.to_owned(),
            passage_sha256: sha256_text(text),
        };
        assert!(!cached_passage_matches(&cached, &identity));
        let cached = CachedPassage {
            book: "new-book".to_owned(),
            ..cached
        };
        assert!(cached_passage_matches(&cached, &identity));
    }

    #[test]
    fn offline_retrieval_without_valid_cache_fails_closed() {
        let entry = access_entry("book");
        let dir = std::env::temp_dir().join(format!(
            "medsynth-passage-cache-test-{}",
            std::process::id()
        ));
        let error = retrieve_passage(&entry, "sec1.1:slug", &dir, true)
            .expect_err("offline retrieval cannot invent a passage");
        assert!(error.to_string().contains("offline mode"));
    }
}
