//! Passive, self-contained HTML from the same bounded selection as Focus mode.
use crate::{focus::Content, render::ImageAsset};
use base64::{Engine, engine::general_purpose::STANDARD};
use html5ever::{
    QualName, ns,
    serialize::{HtmlSerializer, SerializeOpts, Serializer},
};
use olive_html::{Document, NodeId, NodeKind, css::Direction, net::Location};
use std::{
    collections::HashMap,
    io::{self, Write},
};

pub const MAX_EXPORT_BYTES: usize = 8 * 1024 * 1024;
const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 256;

struct BoundedBytes {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedBytes {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(io::Error::other("Focus export exceeds the 8 MiB limit"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn name(local: &str) -> QualName {
    QualName::new(None, ns!(html), local.into())
}
fn attr(local: &str) -> QualName {
    QualName::new(None, ns!(), local.into())
}

pub fn export(
    doc: &Document,
    content: &Content,
    title: &str,
    base: &Location,
    images: &HashMap<NodeId, ImageAsset>,
) -> Result<String, String> {
    export_with_limit(doc, content, title, base, images, MAX_EXPORT_BYTES)
        .map_err(|error| error.to_string())
}

fn export_with_limit(
    doc: &Document,
    content: &Content,
    title: &str,
    base: &Location,
    images: &HashMap<NodeId, ImageAsset>,
    limit: usize,
) -> io::Result<String> {
    let mut out = BoundedBytes {
        bytes: Vec::new(),
        limit,
    };
    let mut serializer = HtmlSerializer::new(&mut out, SerializeOpts::default());
    serializer.write_doctype("html")?;
    serializer.start_elem(
        name("html"),
        [(
            attr("dir"),
            if content.direction == Direction::Rtl {
                "rtl"
            } else {
                "ltr"
            },
        )]
        .iter()
        .map(|(n, v)| (n, *v)),
    )?;
    serializer.start_elem(name("head"), std::iter::empty())?;
    serializer.start_elem(
        name("meta"),
        [(attr("charset"), "utf-8")].iter().map(|(n, v)| (n, *v)),
    )?;
    serializer.end_elem(name("meta"))?;
    // Defense in depth: the allowlisted document has no active or remote content.
    serializer.start_elem(name("meta"), [(attr("http-equiv"), "Content-Security-Policy"), (attr("content"), "default-src 'none'; img-src data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'")].iter().map(|(n,v)| (n,*v)))?;
    serializer.end_elem(name("meta"))?;
    serializer.start_elem(name("title"), std::iter::empty())?;
    serializer.write_text(title)?;
    serializer.end_elem(name("title"))?;
    serializer.start_elem(name("style"), std::iter::empty())?;
    serializer.write_text("body{max-width:720px;margin:2rem auto;padding:0 1rem;font:20px/1.6 sans-serif;color:#2a2e25;background:#faf8f1}img{max-width:100%;height:auto}pre{white-space:pre-wrap}a{color:#4b6327}blockquote{border-inline-start:3px solid #ccd0bf;padding-inline-start:1rem}")?;
    serializer.end_elem(name("style"))?;
    serializer.end_elem(name("head"))?;
    serializer.start_elem(name("body"), std::iter::empty())?;
    if !content.has_heading && !title.is_empty() {
        serializer.start_elem(name("h1"), std::iter::empty())?;
        serializer.write_text(title)?;
        serializer.end_elem(name("h1"))?;
    }
    let mut pending = vec![(content.root, false, 0)];
    let mut nodes = 0;
    let mut image_pixels = 0usize;
    while let Some((id, closing, depth)) = pending.pop() {
        let Some(node) = doc.node(id) else { continue };
        if content.excluded.contains(&id) {
            continue;
        }
        let tag = node.as_element().map(|e| e.name.local.as_str());
        let allowed = tag.is_some_and(|t| {
            matches!(
                t,
                "article"
                    | "main"
                    | "section"
                    | "div"
                    | "p"
                    | "h1"
                    | "h2"
                    | "h3"
                    | "h4"
                    | "h5"
                    | "h6"
                    | "ul"
                    | "ol"
                    | "li"
                    | "dl"
                    | "dt"
                    | "dd"
                    | "blockquote"
                    | "pre"
                    | "code"
                    | "em"
                    | "strong"
                    | "b"
                    | "i"
                    | "u"
                    | "s"
                    | "small"
                    | "sub"
                    | "sup"
                    | "span"
                    | "a"
                    | "br"
                    | "hr"
                    | "figure"
                    | "figcaption"
            )
        });
        if closing {
            if allowed {
                serializer.end_elem(name(tag.unwrap()))?;
            }
            continue;
        }
        nodes += 1;
        if nodes > MAX_NODES || depth > MAX_DEPTH {
            return Err(io::Error::other(
                "Focus export exceeds the document work limit",
            ));
        }
        if let NodeKind::Text(text) = &node.kind {
            serializer.write_text(text)?;
        }
        if let Some(element) = node.as_element() {
            if element.name.ns != ns!(html) {
                continue;
            }
            let tag = tag.unwrap();
            if tag == "img" {
                if let Some(asset) = images
                    .get(&id)
                    .filter(|asset| element.attribute("src") == Some(asset.reference.as_str()))
                {
                    let image = &asset.image;
                    image_pixels = image_pixels.saturating_add(image.pixels.len());
                    if image_pixels > 8 * 1024 * 1024 {
                        return Err(io::Error::other(
                            "Focus export exceeds the image work limit",
                        ));
                    }
                    let mut rgba = Vec::with_capacity(image.pixels.len() * 4);
                    for pixel in &image.pixels {
                        rgba.extend_from_slice(&pixel.to_srgba_unmultiplied());
                    }
                    let mut png = BoundedBytes {
                        bytes: Vec::new(),
                        limit: limit.saturating_mul(3) / 4,
                    };
                    image::ImageEncoder::write_image(
                        image::codecs::png::PngEncoder::new(&mut png),
                        &rgba,
                        image.size[0] as u32,
                        image.size[1] as u32,
                        image::ExtendedColorType::Rgba8,
                    )
                    .map_err(io::Error::other)?;
                    let src = format!("data:image/png;base64,{}", STANDARD.encode(png.bytes));
                    serializer.start_elem(
                        name("img"),
                        [
                            (attr("src"), src.as_str()),
                            (attr("alt"), element.attribute("alt").unwrap_or("")),
                        ]
                        .iter()
                        .map(|(n, v)| (n, *v)),
                    )?;
                    serializer.end_elem(name("img"))?;
                } else {
                    serializer.write_text(element.attribute("alt").unwrap_or(""))?;
                }
                continue;
            }
            if matches!(
                tag,
                "script"
                    | "style"
                    | "template"
                    | "iframe"
                    | "object"
                    | "embed"
                    | "form"
                    | "input"
                    | "button"
                    | "textarea"
                    | "select"
                    | "svg"
                    | "math"
                    | "head"
            ) {
                continue;
            }
            if allowed {
                let mut attrs: Vec<(QualName, String)> = Vec::new();
                for key in ["id", "lang", "dir"] {
                    if let Some(value) = element.attribute(key) {
                        attrs.push((attr(key), value.to_owned()));
                    }
                }
                for key in match tag {
                    "ol" => &["start", "reversed"][..],
                    "li" => &["value"][..],
                    _ => &[],
                } {
                    if let Some(value) = element.attribute(key) {
                        attrs.push((attr(key), value.to_owned()));
                    }
                }
                if tag == "a" {
                    if let Some(href) = element.attribute("href") {
                        if href.starts_with('#') {
                            attrs.push((attr("href"), href.to_owned()));
                        } else if let Ok(location) = base.resolve(href) {
                            if location.is_remote() || !base.is_remote() {
                                attrs.push((attr("href"), location.as_str().to_owned()));
                            }
                        }
                    }
                }
                serializer.start_elem(name(tag), attrs.iter().map(|(n, v)| (n, v.as_str())))?;
                pending.push((id, true, depth));
            }
        }
        let children: Vec<_> = doc.children(id).collect();
        if children.len().saturating_add(pending.len()) > MAX_NODES {
            return Err(io::Error::other(
                "Focus export exceeds the document work limit",
            ));
        }
        pending.extend(
            children
                .into_iter()
                .rev()
                .map(|child| (child, false, depth + 1)),
        );
    }
    serializer.end_elem(name("body"))?;
    serializer.end_elem(name("html"))?;
    String::from_utf8(out.bytes).map_err(io::Error::other)
}

pub fn save(path: &std::path::Path, html: &str) -> io::Result<()> {
    if html.len() > MAX_EXPORT_BYTES {
        return Err(io::Error::other("Focus export exceeds the 8 MiB limit"));
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(html.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use olive_html::{css::Stylesheet, parse};
    fn exported(html: &str, images: &HashMap<NodeId, ImageAsset>) -> String {
        let doc = parse(html).unwrap().document;
        let content = Content::extract(&doc, &Stylesheet::from_document(&doc), false);
        export(
            &doc,
            &content,
            "Reading © שלום",
            &Location::from_input("https://example.com/articles/story").unwrap(),
            images,
        )
        .unwrap()
    }

    #[test]
    fn export_round_trips_unicode_and_passive_article_structure() {
        let output = exported(
            r#"<nav>Navigation</nav><article dir=rtl><h1>שלום © &amp; café</h1><p id=part onclick="alert(1)">© ¢ &lt;script&gt; <strong>Bold</strong></p><ol start=3><li value=7>Seven</li></ol><pre><code>one
  two</code></pre><a href='../next?q=1&amp;x=2' ping='/track' target=_blank>Next</a><a href='#part'>Here</a><a href='javascript:alert(1)'>Unsafe</a><a href='file:///private/test'>File</a><script>alert(1)</script><form><input value=secret></form><div hidden>Hidden</div><img src='/missing' alt='Missing ©'></article>"#,
            &HashMap::new(),
        );
        assert!(!output.contains("Navigation"));
        for banned in [
            "onclick",
            "alert(1)",
            "<script>",
            "<form",
            "secret",
            "Hidden",
            "javascript:",
            "file:",
            "ping=",
            "target=",
            "src=\"/missing",
        ] {
            assert!(!output.contains(banned), "{banned}");
        }
        assert!(output.contains("href=\"https://example.com/next?q=1&amp;x=2\""));
        assert!(output.contains("href=\"#part\""));
        assert!(output.contains("<strong>Bold</strong>"));
        assert!(output.contains("<ol start=\"3\"><li value=\"7\">"));
        let reparsed = parse(&output).unwrap().document;
        let text: String = reparsed
            .descendants(reparsed.root())
            .filter_map(|id| match &reparsed.node(id).unwrap().kind {
                NodeKind::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(text.contains("© ¢ <script>"));
        assert!(text.contains("שלום © & café"));
        assert!(text.contains("one\n  two"));
        assert!(text.contains("Missing ©"));
    }

    #[test]
    fn loaded_images_are_embedded_without_remote_fetches() {
        let doc = parse("<article><p>Image article</p><img src='/photo' alt='Photo ©'></article>")
            .unwrap()
            .document;
        let id = doc
            .descendants(doc.root())
            .find(|id| {
                doc.node(*id)
                    .unwrap()
                    .as_element()
                    .is_some_and(|e| e.name.local.as_str() == "img")
            })
            .unwrap();
        let mut images = HashMap::new();
        images.insert(
            id,
            ImageAsset {
                reference: "/photo".into(),
                image: std::sync::Arc::new(eframe::egui::ColorImage::new(
                    [1, 1],
                    vec![eframe::egui::Color32::RED],
                )),
            },
        );
        let content = Content::extract(&doc, &Stylesheet::from_document(&doc), false);
        let output = export(
            &doc,
            &content,
            "Photo",
            &Location::from_input("https://example.com/").unwrap(),
            &images,
        )
        .unwrap();
        let reparsed = parse(&output).unwrap().document;
        let img = reparsed
            .descendants(reparsed.root())
            .find_map(|id| {
                reparsed
                    .node(id)
                    .unwrap()
                    .as_element()
                    .filter(|e| e.name.local.as_str() == "img")
            })
            .unwrap();
        let png = STANDARD
            .decode(
                img.attribute("src")
                    .unwrap()
                    .strip_prefix("data:image/png;base64,")
                    .unwrap(),
            )
            .unwrap();
        let decoded = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(img.attribute("alt"), Some("Photo ©"));
    }

    #[test]
    fn output_and_depth_limits_fail_without_partial_exports() {
        let html = format!("<article><p>{}</p></article>", "&".repeat(2000));
        let doc = parse(&html).unwrap().document;
        let content = Content::extract(&doc, &Stylesheet::from_document(&doc), false);
        let base = Location::from_input("https://example.com/").unwrap();
        assert!(export_with_limit(&doc, &content, "Title", &base, &HashMap::new(), 2048).is_err());
        let doc = parse(&format!(
            "<article>{}<p>Deep</p>{}</article>",
            "<div>".repeat(300),
            "</div>".repeat(300)
        ))
        .unwrap()
        .document;
        let content = Content::extract(&doc, &Stylesheet::from_document(&doc), false);
        assert!(
            export(&doc, &content, "Deep", &base, &HashMap::new())
                .unwrap_err()
                .contains("work limit")
        );
    }

    #[test]
    fn save_replaces_atomically_and_failures_preserve_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("article.html");
        std::fs::write(&path, "original").unwrap();
        assert!(save(&path, &"x".repeat(MAX_EXPORT_BYTES + 1)).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        save(&path, "<p>Saved ©</p>").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "<p>Saved ©</p>");
        assert!(save(&dir.path().join("missing/article.html"), "new").is_err());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
