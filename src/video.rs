//! Lesson videos. Only https links to hosts that serve embeddable players — YouTube, Vimeo, Loom,
//! Wistia, Google Drive, plus hosts the workspace allows in Learn Settings — are accepted, and each
//! becomes the URL of its player, so a lesson page frames a known player instead of trusting
//! whatever markup an author pasted. Parsing uses the `url` crate, so tricks like
//! `https://youtube.com@evil.test/` or `https://youtube.com.evil.test/` name the host they really go to.

use url::{Host, Url};

const MAX_URL: usize = 2048;

fn token(v: &str, min: usize, max: usize) -> Option<&str> {
    (v.len() >= min && v.len() <= max && v.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')).then_some(v)
}

/// `90`, `90s`, `1m30s`, `1h2m3s` → seconds.
fn seconds(v: &str) -> Option<u32> {
    if let Ok(n) = v.parse::<u32>() {
        return Some(n);
    }
    let (mut total, mut num) = (0u32, String::new());
    for c in v.chars() {
        match c {
            '0'..='9' => num.push(c),
            'h' | 'm' | 's' => {
                let n: u32 = num.parse().ok()?;
                total = total.checked_add(n.checked_mul(match c {
                    'h' => 3600,
                    'm' => 60,
                    _ => 1,
                })?)?;
                num.clear();
            }
            _ => return None,
        }
    }
    num.is_empty().then_some(total)
}

fn query<'a>(url: &'a Url, key: &str) -> Option<std::borrow::Cow<'a, str>> {
    url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v)
}

fn youtube(id: Option<&str>, url: &Url) -> Result<String, String> {
    let id = id.and_then(|i| token(i, 11, 11)).ok_or("is not a link to a YouTube video")?;
    let start = query(url, "t").or_else(|| query(url, "start")).and_then(|t| seconds(&t)).filter(|s| *s > 0);
    Ok(match start {
        Some(s) => format!("https://www.youtube-nocookie.com/embed/{id}?start={s}"),
        None => format!("https://www.youtube-nocookie.com/embed/{id}"),
    })
}

fn is_or_under(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// The player URL for a lesson's `video_url`, or why the link is refused.
pub fn embed_url(raw: &str, extra_hosts: &[String]) -> Result<String, String> {
    let raw = raw.trim();
    if raw.len() > MAX_URL {
        return Err("is too long".into());
    }
    let url = Url::parse(raw).map_err(|_| "is not a valid link".to_string())?;
    if url.scheme() != "https" {
        return Err("must be an https link".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("must not contain a user name or password".into());
    }
    let host = match url.host() {
        Some(Host::Domain(h)) => h.trim_end_matches('.').to_ascii_lowercase(),
        _ => return Err("must name a host, not an IP address".into()),
    };
    let segs: Vec<&str> = url.path_segments().map(|p| p.filter(|x| !x.is_empty()).collect()).unwrap_or_default();
    let known_port = url.port().is_none();
    match host.as_str() {
        "youtube.com" | "www.youtube.com" | "m.youtube.com" | "youtube-nocookie.com" | "www.youtube-nocookie.com" if known_port => {
            let id = match segs.as_slice() {
                ["watch"] => query(&url, "v").map(|v| v.into_owned()),
                ["embed" | "shorts" | "live" | "v", id] => Some(id.to_string()),
                _ => None,
            };
            youtube(id.as_deref(), &url)
        }
        "youtu.be" if known_port => youtube(segs.first().copied(), &url),
        "vimeo.com" | "www.vimeo.com" | "player.vimeo.com" if known_port => {
            let numeric = |v: &str| v.len() <= 20 && !v.is_empty() && v.bytes().all(|c| c.is_ascii_digit());
            let (id, hash) = match segs.as_slice() {
                ["video", id, ..] if host == "player.vimeo.com" => (Some(*id), None),
                [id] => (Some(*id), None),
                [id, hash] if numeric(id) => (Some(*id), Some(*hash)),
                ["channels", _, id] | ["groups", _, "videos", id] => (Some(*id), None),
                _ => (None, None),
            };
            let id = id.filter(|i| numeric(i)).ok_or("is not a link to a Vimeo video")?;
            let hash = hash.map(str::to_string).or_else(|| query(&url, "h").map(|h| h.into_owned()));
            match hash {
                Some(h) => Ok(format!("https://player.vimeo.com/video/{id}?h={}", token(&h, 4, 40).ok_or("has an invalid Vimeo privacy hash")?)),
                None => Ok(format!("https://player.vimeo.com/video/{id}")),
            }
        }
        "loom.com" | "www.loom.com" if known_port => match segs.as_slice() {
            ["share" | "embed", id] => Ok(format!("https://www.loom.com/embed/{}", token(id, 8, 64).ok_or("is not a link to a Loom video")?)),
            _ => Err("is not a link to a Loom video".into()),
        },
        h if known_port && (is_or_under(h, "wistia.com") || is_or_under(h, "wistia.net")) => {
            let id = match segs.as_slice() {
                ["medias", id] | ["embed", "iframe", id] | ["embed", "medias", id] => token(id, 6, 32),
                _ => None,
            };
            Ok(format!("https://fast.wistia.net/embed/iframe/{}", id.ok_or("is not a link to a Wistia video")?))
        }
        "drive.google.com" if known_port => match segs.as_slice() {
            ["file", "d", id, ..] => {
                Ok(format!("https://drive.google.com/file/d/{}/preview", token(id, 10, 100).ok_or("is not a link to a Google Drive file")?))
            }
            _ => Err("is not a link to a Google Drive file (use the file's share link)".into()),
        },
        h if extra_hosts.iter().any(|x| is_or_under(h, x)) => Ok(url.to_string()),
        _ => Err("is not on an allowed video host (YouTube, Vimeo, Loom, Wistia, Google Drive or a host allowed in Learn Settings)".into()),
    }
}

/// The hosts listed in Learn Settings (one per line, `*.` prefixes ignored), lower-case.
pub fn parse_hosts(text: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
        let host = line.trim_start_matches("*.").trim_end_matches('.').to_ascii_lowercase();
        let ok = host.len() <= 253
            && host.contains('.')
            && host.split('.').all(|l| {
                !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-') && l.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            });
        if !ok {
            return Err(format!("'{line}' is not a host name (write one host per line, e.g. video.example.com)"));
        }
        if !out.contains(&host) {
            out.push(host);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(raw: &str) -> String {
        embed_url(raw, &[]).unwrap_or_else(|e| panic!("{raw}: {e}"))
    }

    #[test]
    fn known_players_become_embed_urls() {
        let yt = "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ";
        assert_eq!(ok("https://www.youtube.com/watch?v=dQw4w9WgXcQ"), yt);
        assert_eq!(ok("https://youtu.be/dQw4w9WgXcQ"), yt);
        assert_eq!(ok("https://m.youtube.com/shorts/dQw4w9WgXcQ"), yt);
        assert_eq!(ok("https://youtu.be/dQw4w9WgXcQ?t=1m30s"), format!("{yt}?start=90"));
        assert_eq!(ok("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42"), format!("{yt}?start=42"));
        assert_eq!(ok("https://vimeo.com/76979871"), "https://player.vimeo.com/video/76979871");
        assert_eq!(ok("https://vimeo.com/76979871/a1b2c3d4e5"), "https://player.vimeo.com/video/76979871?h=a1b2c3d4e5");
        assert_eq!(ok("https://player.vimeo.com/video/76979871?h=abc123"), "https://player.vimeo.com/video/76979871?h=abc123");
        assert_eq!(ok("https://www.loom.com/share/0123456789abcdef0123456789abcdef"), "https://www.loom.com/embed/0123456789abcdef0123456789abcdef");
        assert_eq!(ok("https://acme.wistia.com/medias/e4a27b971d"), "https://fast.wistia.net/embed/iframe/e4a27b971d");
        assert_eq!(ok("https://drive.google.com/file/d/1AbCdEfGhIjKlMnOp/view?usp=sharing"), "https://drive.google.com/file/d/1AbCdEfGhIjKlMnOp/preview");
    }

    #[test]
    fn everything_else_is_refused() {
        for bad in [
            "http://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://youtube.com.evil.test/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com@evil.test/watch?v=dQw4w9WgXcQ",
            "https://evil.test/?u=https://youtu.be/dQw4w9WgXcQ",
            "https://www.youtube.com/watch?v=short",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ<script>",
            "https://www.youtube.com:8443/watch?v=dQw4w9WgXcQ",
            "https://93.184.216.34/video.mp4",
            "https://vimeo.com/channels/staffpicks",
            "javascript:alert(1)",
            "https://notwistia.com/medias/e4a27b971d",
            "not a url",
        ] {
            assert!(embed_url(bad, &[]).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn workspace_hosts() {
        let hosts = parse_hosts("# our servers\nvideo.example.com\n*.cdn.example.org\n").unwrap();
        assert_eq!(hosts, vec!["video.example.com".to_string(), "cdn.example.org".to_string()]);
        assert_eq!(embed_url("https://video.example.com/v/42?x=1", &hosts).unwrap(), "https://video.example.com/v/42?x=1");
        assert!(embed_url("https://eu.cdn.example.org/a.mp4", &hosts).is_ok());
        assert!(embed_url("https://video.example.com.evil.test/a", &hosts).is_err());
        assert!(embed_url("http://video.example.com/v/42", &hosts).is_err());
        assert!(parse_hosts("not a host").is_err() && parse_hosts("-bad-.example.com").is_err());
        assert_eq!(seconds("1h2m3s"), Some(3723));
        assert_eq!(seconds("1x"), None);
    }
}
