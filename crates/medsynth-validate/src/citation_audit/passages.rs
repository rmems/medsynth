//! Passage retrieval for citation-support audit: cache handling, OpenStax
//! locator resolution, fetching, and HTML text extraction.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

use super::{AccessEntry, sha256_text};

#[derive(Debug, Clone)]
pub(crate) struct Passage {
    pub(crate) url: String,
    pub(crate) text: String,
    pub(crate) sha256: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct CachedPassage {
    source_id: String,
    locator: String,
    url: String,
    text: String,
    passage_sha256: String,
}

fn cache_path(cache_dir: &Path, source_id: &str, locator: &str) -> PathBuf {
    cache_dir
        .join(source_id)
        .join(format!("{}.json", sha256_text(locator)))
}

fn read_cached_passage(path: &Path, source_id: &str, locator: &str) -> Option<Passage> {
    let text = fs::read_to_string(path).ok()?;
    let cached: CachedPassage = serde_json::from_str(&text).ok()?;
    if cached.source_id != source_id || cached.locator != locator {
        return None;
    }
    if cached.passage_sha256 != sha256_text(&cached.text) {
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
    source_id: &str,
    locator: &str,
    passage: &Passage,
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let cached = CachedPassage {
        source_id: source_id.to_owned(),
        locator: locator.to_owned(),
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

fn page_candidate(file: &str, chapter: &str, slug: &str) -> bool {
    let Some(rest) = file.strip_prefix(&format!("{chapter}-")) else {
        return false;
    };
    let Some(section) = rest.strip_suffix(&format!("-{slug}")) else {
        return false;
    };
    !section.is_empty() && section.chars().all(|c| c.is_ascii_digit())
}

/// Find `chapter-section-slug` in an OpenStax chapter introduction page.
pub fn find_openstax_page_path(
    index_html: &str,
    book: &str,
    chapter: &str,
    slug: &str,
) -> Option<String> {
    let document = Html::parse_document(index_html);
    let selector = Selector::parse("a[href]").ok()?;
    document
        .select(&selector)
        .filter_map(|element| element.value().attr("href"))
        .filter_map(|href| {
            let clean = href.split('#').next()?.trim_end_matches('/');
            let file = clean.rsplit('/').next()?;
            page_candidate(file, chapter, slug)
                .then(|| format!("https://openstax.org/books/{book}/pages/{file}"))
        })
        .next()
}

fn direct_section_url(book: &str, locator: &str) -> Option<String> {
    let (prefix, slug) = locator.split_once(':')?;
    let section = prefix.strip_prefix("sec")?;
    let (chapter, section) = section.split_once('.')?;
    if chapter.is_empty()
        || section.is_empty()
        || !chapter.chars().all(|c| c.is_ascii_digit())
        || !section.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some(format!(
        "https://openstax.org/books/{book}/pages/{chapter}-{section}-{slug}"
    ))
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
    if let Some(url) = direct_section_url(&book, locator) {
        return Ok(url);
    }
    let (chapter, slug) = chapter_locator(locator)
        .ok_or_else(|| anyhow::anyhow!("unsupported OpenStax locator `{locator}`"))?;
    if offline {
        bail!("locator `{locator}` requires chapter-index resolution and no cached passage exists");
    }
    let index_url = format!("https://openstax.org/books/{book}/pages/{chapter}-introduction");
    let index = fetch_text(&index_url)?;
    find_openstax_page_path(&index, &book, &chapter, &slug)
        .ok_or_else(|| anyhow::anyhow!("locator `{locator}` not found in {index_url}"))
}

/// Extract the visible text of OpenStax's `<main>` content.
pub fn extract_main_text(html: &str) -> String {
    let document = Html::parse_document(html);
    let selector = Selector::parse("main").expect("main selector is valid");
    let text = document
        .select(&selector)
        .next()
        .map(|main| main.text().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(crate) fn retrieve_passage(
    entry: &AccessEntry,
    locator: &str,
    cache_dir: &Path,
    offline: bool,
) -> anyhow::Result<Passage> {
    let path = cache_path(cache_dir, &entry.source_id, locator);
    if let Some(passage) = read_cached_passage(&path, &entry.source_id, locator) {
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
    write_cached_passage(&path, &entry.source_id, locator, &passage)?;
    Ok(passage)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_visible_openstax_main_text() {
        let html = r#"<!doctype html><html><body><nav>skip me</nav><main><h1>Section</h1><p>Gap junctions connect cells.</p></main></body></html>"#;
        assert_eq!(
            extract_main_text(html),
            "Section Gap junctions connect cells."
        );
    }

    #[test]
    fn resolves_chapter_slug_from_index_links() {
        let html = r#"<html><body><a href="19-3-health-in-the-united-states">x</a></body></html>"#;
        assert_eq!(
            find_openstax_page_path(
                html,
                "introduction-sociology-3e",
                "19",
                "health-in-the-united-states"
            ),
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
            "1",
            "health-in-the-united-states"
        ));
        assert!(!page_candidate(
            "19-3-health-in-the-united-states-extra",
            "19",
            "health-in-the-united-states"
        ));
    }

    #[test]
    fn direct_section_locator_maps_to_page_url() {
        assert_eq!(
            direct_section_url("anatomy-and-physiology-2e", "sec4.2:epithelial-tissue"),
            Some(
                "https://openstax.org/books/anatomy-and-physiology-2e/pages/4-2-epithelial-tissue"
                    .to_owned()
            )
        );
    }
}
