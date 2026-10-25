//! Entry points for the fuzz targets in `fuzz/` and for the smoke test that
//! runs them in every build. Each takes arbitrary bytes and must not panic,
//! hang or allocate without bound; errors are ordinary outcomes. Not a stable
//! API.

/// HTML parsing, as for every document.
pub fn html(data: &[u8]) {
    if let Ok(output) = crate::parse_utf8(data, crate::ParseOptions::default()) {
        let document = output.document;
        for id in document.descendants(document.root()) {
            let _ = document.node(id);
        }
    }
}

/// Stylesheet parsing and matching against a small document.
#[cfg(feature = "css")]
pub fn css(data: &[u8]) {
    use crate::css::{ComputedStyle, StyleBudget, Stylesheet};
    use std::collections::HashMap;
    let sheet = Stylesheet::parse(&String::from_utf8_lossy(data));
    let Ok(output) = crate::parse(
        "<div id=a class='b c' style='--x: 1px; width: calc(var(--x) * 2)'><p>t<span lang=en>s</span></p><ul><li>1<li>2</ul><table><tr><td>c</table></div>",
    ) else {
        return;
    };
    let document = output.document;
    let mut budget = StyleBudget::default();
    let mut styles: HashMap<_, ComputedStyle> = HashMap::new();
    for id in document.descendants(document.root()) {
        let parent = document
            .node(id)
            .and_then(|node| node.parent())
            .and_then(|parent| styles.get(&parent))
            .cloned()
            .unwrap_or_default();
        let style = sheet.compute(&document, id, parent, 17.0, &mut budget);
        styles.insert(id, style);
    }
}

/// Cookie parsing and selection: each line is a `Set-Cookie` value, applied
/// in turn as a response's and a script's for one of several sites.
#[cfg(feature = "net")]
pub fn cookies(data: &[u8]) {
    use crate::net::{CookieJar, CookieUpdate, Location};
    let sites = [
        "https://www.example.com/docs/page",
        "http://example.com/",
        "https://cdn.example.co.uk/a/b",
        "http://127.0.0.1:8080/",
    ]
    .map(|address| Location::from_input(address).unwrap());
    let text = String::from_utf8_lossy(data);
    let mut jar = CookieJar::default();
    for (index, line) in text.lines().take(64).enumerate() {
        let site = &sites[index % sites.len()];
        let mut update = CookieUpdate::from_script(site.clone(), line.to_owned());
        update.from_script = index % 2 == 0;
        jar.apply_updates(&[update]);
        jar.set_document_cookie(site, line);
    }
    for site in &sites {
        let _ = jar.cookie_header(site);
        let _ = jar.document_cookie(site);
        let _ = jar.for_document(site).cookie_header(site);
        let _ = jar.for_script(site).document_cookie(site);
    }
}

/// Address-bar input and link resolution: the first line is an address and
/// the rest a link on its page.
#[cfg(feature = "net")]
pub fn location(data: &[u8]) {
    use crate::net::Location;
    let text = String::from_utf8_lossy(data);
    let (address, link) = text.split_once('\n').unwrap_or((&text, ""));
    if let Ok(location) = Location::from_input(address) {
        let _ = location.resolve(link);
        let _ = location.fragment();
    }
    let page = Location::from_input("https://example.com/dir/page").unwrap();
    let _ = page.resolve(&text);
}

/// The browser's reading of a tab process's message: framing, JSON, raw
/// pictures and the presentation checks.
#[cfg(feature = "gui")]
pub fn tab_message(data: &[u8]) {
    let _ = crate::gui::worker::read_event(&mut &data[..]);
}

/// The browser's reading of a network process's message.
#[cfg(feature = "gui")]
pub fn network_message(data: &[u8]) {
    let _ = crate::gui::network::check_event(data);
}

/// Picture decoding in tab processes, as page images and site icons.
#[cfg(feature = "gui")]
pub fn image(data: &[u8]) {
    let _ = crate::gui::document::check_image(data);
}

/// A fuzz target's name and entry point.
pub type Target = (&'static str, fn(&[u8]));

/// The targets this build has, by fuzz target name.
pub fn targets() -> Vec<Target> {
    vec![
        ("html", html as fn(&[u8])),
        #[cfg(feature = "css")]
        ("css", css),
        #[cfg(feature = "net")]
        ("cookies", cookies),
        #[cfg(feature = "net")]
        ("location", location),
        #[cfg(feature = "gui")]
        ("tab_message", tab_message),
        #[cfg(feature = "gui")]
        ("network_message", network_message),
        #[cfg(feature = "gui")]
        ("image", image),
    ]
}

/// Well-formed inputs for the target `name`, as starting points for fuzzing.
pub fn seeds(name: &str) -> Vec<Vec<u8>> {
    let text = |samples: &[&str]| {
        samples
            .iter()
            .map(|sample| sample.as_bytes().to_vec())
            .collect()
    };
    match name {
        "html" => text(&[
            "<!doctype html><title>x</title><p class=a>b<table><tr><td>c</table>",
            "<svg><foreignObject><math><mi><template><select><option>",
            "<meta charset=windows-1255><p>\u{5e9}\u{5dc}\u{5d5}\u{5dd}&amp;&#x1F600;",
        ]),
        "css" => text(&[
            "div.b > p:not(.x) span[lang|=en] { color: rgb(1 2 3 / 50%); margin: 1em auto }",
            "@media screen and (min-width: 600px) { ul li:nth-child(2n+1) { display: flex } }",
            ":is(#a, .c) :where(td) { width: calc(100% - var(--x, 2px)); font: 12px/1.5 serif }",
        ]),
        "cookies" => text(&[
            "a=1; Path=/docs; HttpOnly\nwide=2; Domain=example.com; Max-Age=60\n__Host-x=1; Secure; Path=/",
            "s=1; SameSite=None; Secure; Expires=Wed, 09 Jun 2021 10:18:14 GMT\nbad=1; Domain=co.uk",
        ]),
        "location" => text(&[
            "example.com/docs/index.html\n../about?q=olive tree#one",
            "localhost:8000/test\n//other.example/path",
            "https://[::1]:3000/a\nfile:///etc/passwd",
            "/notes/c:\n../next.html",
        ]),
        #[cfg(feature = "gui")]
        "tab_message" => {
            use crate::gui::worker::{Event, write_frame};
            let source = crate::net::LoadedDocument {
                location: crate::net::Location::from_input("https://example.com/").unwrap(),
                bytes: b"<title>T</title><p>Text <b>bold</b><img alt=y><form><input name=q></form>"
                    .to_vec(),
                status: Some(200),
                plain_text: false,
            };
            let (page, _) =
                crate::gui::document::prepare_page_preview(&source, false, (800.0, 600.0)).unwrap();
            [
                Event::Loaded(Box::new(page)),
                Event::Error("Could not load".into()),
            ]
            .iter()
            .map(|event| {
                let mut frame = Vec::new();
                write_frame(&mut frame, event, 128 * 1024 * 1024).unwrap();
                frame
            })
            .collect()
        }
        #[cfg(feature = "gui")]
        "network_message" => crate::gui::network::sample_events(),
        #[cfg(feature = "gui")]
        "image" => {
            use ::image::{ImageFormat, Rgba, RgbaImage};
            let picture =
                RgbaImage::from_fn(3, 2, |x, y| Rgba([x as u8 * 80, y as u8 * 120, 7, 255]));
            let encode = |format| {
                let mut bytes = std::io::Cursor::new(Vec::new());
                picture.write_to(&mut bytes, format).unwrap();
                bytes.into_inner()
            };
            vec![
                encode(ImageFormat::Png),
                encode(ImageFormat::Gif),
                encode(ImageFormat::Ico),
                include_bytes!("../tests/fixtures/images/red-2x1.jpg").to_vec(),
            ]
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixed-seed mutator: enough to catch panics in every build without a
    /// fuzzing toolchain. The fuzz targets explore much further.
    struct Mutator(u64);

    impl Mutator {
        fn next(&mut self) -> u64 {
            // xorshift64
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, bound: usize) -> usize {
            (self.next() % bound.max(1) as u64) as usize
        }

        fn mutate(&mut self, seed: &[u8]) -> Vec<u8> {
            let mut data = seed.to_vec();
            for _ in 0..=self.below(8) {
                let at = self.below(data.len());
                match self.below(5) {
                    0 if !data.is_empty() => data[at] ^= 1 << self.below(8),
                    1 if !data.is_empty() => data[at] = self.next() as u8,
                    2 => data.insert(self.below(data.len() + 1), self.next() as u8),
                    3 if !data.is_empty() => data.truncate(at),
                    _ => {
                        let at = self.below(data.len() + 1);
                        let extreme: &[u8] = match self.below(3) {
                            0 => &[0xff, 0xff, 0xff, 0x7f],
                            1 => &[0, 0, 0, 0],
                            _ => b"\\u0000\"}]",
                        };
                        data.splice(at..at, extreme.iter().copied());
                    }
                }
            }
            data
        }
    }

    #[test]
    fn every_target_survives_its_seeds_and_their_mutations() {
        let mut mutator = Mutator(0x9e37_79b9_7f4a_7c15);
        for (name, target) in targets() {
            let seeds = seeds(name);
            assert!(!seeds.is_empty(), "{name} has no seeds");
            for seed in &seeds {
                target(seed);
            }
            // Messages with pictures are larger and slower to mutate.
            let rounds = if name == "tab_message" { 300 } else { 1_500 };
            for round in 0..rounds {
                target(&mutator.mutate(&seeds[round % seeds.len()]));
            }
        }
    }

    /// Write the seeds to `fuzz/corpus/<target>/seed-<n>`:
    /// `cargo test --all-features write_fuzz_corpus -- --ignored`.
    #[test]
    #[ignore]
    fn write_fuzz_corpus() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus");
        for (name, _) in targets() {
            let directory = root.join(name);
            std::fs::create_dir_all(&directory).unwrap();
            for (index, seed) in seeds(name).iter().enumerate() {
                std::fs::write(directory.join(format!("seed-{index}")), seed).unwrap();
            }
        }
    }
}
