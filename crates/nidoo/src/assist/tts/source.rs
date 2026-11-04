//! Reading the address a person pastes for a model.

use url::Url;

use super::TtsError;

/// A Hugging Face model repository, optionally pinned to a revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HfSource {
    /// The `owner/repo` name.
    pub repo: String,
    /// The branch, tag or commit, when the address names one.
    pub rev: Option<String>,
}

fn bad() -> TtsError {
    TtsError::Invalid("not a Hugging Face model address".into())
}

fn valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphanumeric())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        && !s.contains("..")
}

fn valid_rev(s: &str) -> bool {
    !s.is_empty()
        && !s.contains("..")
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Reads `https://huggingface.co/owner/repo[/tree|blob|resolve/rev/...]`, `hf://owner/repo[@rev]`
/// or a bare `owner/repo[@rev]`; anything else is refused.
pub fn parse_source(input: &str) -> Result<HfSource, TtsError> {
    let input = input.trim();
    let (path, mut rev): (String, Option<String>) =
        if input.starts_with("http://") || input.starts_with("https://") {
            let url = Url::parse(input).map_err(|_| bad())?;
            if url.scheme() != "https"
                || url.host_str().map(str::to_ascii_lowercase).as_deref() != Some("huggingface.co")
            {
                return Err(bad());
            }
            (url.path().to_string(), None)
        } else if let Some(rest) = input.strip_prefix("hf://") {
            (rest.to_string(), None)
        } else if input.contains("://") {
            return Err(bad());
        } else {
            (input.to_string(), None)
        };
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let (owner, rest) = match segments.as_slice() {
        [owner, rest @ ..] => (*owner, rest),
        [] => return Err(bad()),
    };
    if matches!(owner, "datasets" | "spaces") {
        return Err(bad());
    }
    let (mut repo, mut rest) = match rest {
        [repo, rest @ ..] => (*repo, rest),
        [] => return Err(bad()),
    };
    if let Some((r, v)) = repo.split_once('@') {
        repo = r;
        rev = Some(v.to_string());
    }
    if let [kind, r, ..] = rest
        && matches!(*kind, "tree" | "blob" | "resolve")
    {
        rev = Some((*r).to_string());
        rest = &[];
    }
    // Any other trailing segment (`gradium/x/y`) is not a model address.
    if !rest.is_empty() || !valid_name(owner) || !valid_name(repo) {
        return Err(bad());
    }
    if let Some(r) = &rev
        && !valid_rev(r)
    {
        return Err(bad());
    }
    Ok(HfSource {
        repo: format!("{owner}/{repo}"),
        rev,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_the_shapes_people_paste() {
        for s in [
            "https://huggingface.co/gradium/phonon-40m-fr",
            "https://huggingface.co/gradium/phonon-40m-fr/",
            "https://HuggingFace.co/gradium/phonon-40m-fr/tree/main",
            "https://huggingface.co/gradium/phonon-40m-fr/blob/main/model.q8.gguf",
            "https://huggingface.co/gradium/phonon-40m-fr/resolve/main/config.json?download=true",
            "hf://gradium/phonon-40m-fr",
            "gradium/phonon-40m-fr",
            "  gradium/phonon-40m-fr  ",
        ] {
            assert_eq!(
                parse_source(s).unwrap().repo,
                "gradium/phonon-40m-fr",
                "{s}"
            );
        }
    }

    #[test]
    fn keeps_a_pinned_revision() {
        assert_eq!(
            parse_source("hf://gradium/x@abc123")
                .unwrap()
                .rev
                .as_deref(),
            Some("abc123")
        );
        assert_eq!(
            parse_source("https://huggingface.co/gradium/x/tree/abc123")
                .unwrap()
                .rev
                .as_deref(),
            Some("abc123")
        );
    }

    #[test]
    fn refuses_other_hosts_and_shapes() {
        for s in [
            "http://evil.example/gradium/x",
            "https://huggingface.co.evil.example/gradium/x",
            "https://evil.example/huggingface.co/gradium/x",
            "https://huggingface.co/datasets/gradium/x",
            "gradium",
            "gradium/x/y",
            "../etc/passwd",
            "",
        ] {
            assert!(parse_source(s).is_err(), "{s}");
        }
    }
}
