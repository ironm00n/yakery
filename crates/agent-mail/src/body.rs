//! A mail body from Markdown: the Markdown itself as text/plain, and its rendering as text/html.

use anyhow::Result;
use comrak::{Options, markdown_to_html};
use lol_html::{RewriteStrSettings, element, rewrite_str};

use crate::signature::Signature;

/// Mail clients drop `<style>` blocks unevenly, so styles go inline. The grey tints read on light and dark themes.
/// A later selector replaces an earlier one's style, so `pre > code` drops the inline-code background.
/// The tints are `background-color`: Bulwark Mobile's dark mode re-inverts anything whose inline style holds
/// `background:`, which leaves the inherited text dark on its dark page.
const STYLES: &[(&str, &str)] = &[
    (
        "pre",
        "padding: 8px 12px; border-radius: 4px; background-color: rgba(127, 127, 127, 0.12); overflow-x: auto",
    ),
    (
        "code",
        "font-family: ui-monospace, Menlo, Consolas, monospace; padding: 1px 4px; border-radius: 3px; background-color: rgba(127, 127, 127, 0.15)",
    ),
    (
        "pre > code",
        "font-family: ui-monospace, Menlo, Consolas, monospace",
    ),
    ("table", "border-collapse: collapse"),
    (
        "th",
        "border: 1px solid rgba(127, 127, 127, 0.5); padding: 4px 8px",
    ),
    (
        "td",
        "border: 1px solid rgba(127, 127, 127, 0.5); padding: 4px 8px",
    ),
    (
        "blockquote",
        "margin: 0; padding-left: 12px; border-left: 3px solid rgba(127, 127, 127, 0.5)",
    ),
];

pub struct Body {
    pub text: String,
    pub html: String,
}

impl Body {
    pub fn new(markdown: &str, signature: &Signature) -> Result<Body> {
        let text = format!("{}\n\n{}\n", markdown.trim_end(), signature.text());
        let html = format!(
            "<!DOCTYPE html>\n<html>\n<head><meta charset=\"utf-8\"></head>\n<body>\n{}{}</body>\n</html>\n",
            render(markdown),
            signature.html()
        );
        Ok(Body {
            text,
            html: style(&html)?,
        })
    }
}

fn render(markdown: &str) -> String {
    let mut options = Options::default();
    options.extension.table = true;
    options.extension.strikethrough = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.render.escape = true;
    markdown_to_html(markdown, &options)
}

fn style(html: &str) -> Result<String> {
    let settings = STYLES
        .iter()
        .fold(RewriteStrSettings::new(), |settings, &(selector, style)| {
            settings.append_element_content_handler(element!(selector, move |el| {
                el.set_attribute("style", style)?;
                Ok(())
            }))
        });
    Ok(rewrite_str(html, settings)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html(markdown: &str) -> String {
        Body::new(markdown, &Signature::new("desktop", None))
            .unwrap()
            .html
    }

    #[test]
    fn plain_part_is_the_markdown_and_the_signature() {
        let body = Body::new("# hi\n\n**there**\n\n\n", &Signature::new("desktop", None)).unwrap();
        assert_eq!(
            body.text,
            "# hi\n\n**there**\n\n-- \nSent from a shell on desktop, not from a Claude Code session.\n"
        );
    }

    #[test]
    fn links_render_and_bare_urls_autolink() {
        let html =
            html("see [the page](https://artifacts.ironmoon.dev/x) and https://ironmoon.dev");
        assert!(
            html.contains(r#"<a href="https://artifacts.ironmoon.dev/x">the page</a>"#),
            "{html}"
        );
        assert!(
            html.contains(r#"<a href="https://ironmoon.dev">https://ironmoon.dev</a>"#),
            "{html}"
        );
    }

    #[test]
    fn a_bare_url_stops_before_trailing_punctuation() {
        let url = "https://artifacts.ironmoon.dev/claude/v6.pdf";
        let link = format!(r#"<a href="{url}">{url}</a>"#);
        for (markdown, after) in [
            (
                format!("v6 attached ({url}): the new flow"),
                "): the new flow",
            ),
            (format!("the deck: {url}."), "."),
            (format!("see {url}, then reply"), ", then reply"),
        ] {
            let html = html(&markdown);
            assert!(
                html.contains(&format!("{link}{after}")),
                "{markdown} gave {html}"
            );
        }
    }

    #[test]
    fn tables_render_with_inline_styles() {
        let html = html("| a | b |\n|---|--:|\n| 1 | 2 |\n");
        assert!(
            html.contains(r#"<table style="border-collapse: collapse">"#),
            "{html}"
        );
        assert!(html.contains(r#"<th style="border: 1px solid"#), "{html}");
        assert!(
            html.contains(r#"<td align="right" style="border: 1px solid"#),
            "{html}"
        );
    }

    #[test]
    fn raw_html_is_escaped() {
        let html = html("<script>alert(1)</script>\n\nand <b>inline</b>");
        assert!(!html.contains("<script>"), "{html}");
        assert!(!html.contains("<b>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn code_blocks_and_inline_code_get_their_own_styles() {
        let html = html("```rust\nfn main() {}\n```\n\nrun `ls`\n\n> quoted\n");
        assert!(html.contains(r#"<pre style="padding: 8px 12px"#), "{html}");
        assert!(
            html.contains(r#"<code class="language-rust" style="font-family: ui-monospace, Menlo, Consolas, monospace">"#),
            "{html}"
        );
        assert!(html.contains(r#"<code style="font-family: ui-monospace, Menlo, Consolas, monospace; padding: 1px 4px"#), "{html}");
        assert!(html.contains(r#"<blockquote style="margin: 0"#), "{html}");
    }

    #[test]
    fn no_inline_style_uses_the_background_shorthand() {
        for (selector, style) in STYLES {
            assert!(!style.contains("background:"), "{selector}: {style}");
        }
    }

    #[test]
    fn strikethrough_and_tasklists() {
        let html = html("~~gone~~\n\n- [x] done\n- [ ] todo\n");
        assert!(html.contains("<del>gone</del>"), "{html}");
        assert!(html.contains(r#"type="checkbox""#), "{html}");
    }
}
