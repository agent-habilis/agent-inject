//! Turn a name the phone sent into a safe basename inside the target dir.
//!
//! The name is untrusted: it comes off the network from whoever holds the
//! ticket. Only the last path component survives, so `../../x` and `C:\x`
//! cannot leave the directory, and leading dots are stripped so an upload
//! can never be hidden or become `..`.

/// Longest basename most filesystems accept.
const MAX_BYTES: usize = 255;

/// An extension longer than this is treated as part of the stem when the name
/// has to be cut.
const MAX_EXTENSION_BYTES: usize = 16;

/// Fallback when nothing usable is left.
const FALLBACK: &str = "upload";

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// A basename that is safe to join onto the target directory.
pub(crate) fn sanitize(raw: &str) -> String {
    let last = raw.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last
        .chars()
        .filter(|ch| !ch.is_control())
        .map(|ch| if "<>:\"|?*".contains(ch) { '_' } else { ch })
        .collect();
    let trimmed = cleaned
        .trim()
        .trim_end_matches(['.', ' '])
        .trim_start_matches('.');
    if trimmed.is_empty() {
        return FALLBACK.to_owned();
    }
    let (stem, extension) = split_extension(trimmed);
    let stem = if WINDOWS_RESERVED
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(stem))
    {
        format!("_{stem}")
    } else {
        stem.to_owned()
    };
    fit(&stem, "", extension)
}

/// `name`, then `name-2`, `name-3`, … with the suffix before the extension.
pub(crate) fn candidates(name: &str) -> impl Iterator<Item = String> + '_ {
    let (stem, extension) = split_extension(name);
    std::iter::once(name.to_owned())
        .chain((2u32..).map(move |index| fit(stem, &format!("-{index}"), extension)))
}

/// Split off the last `.ext` when it is short enough to be an extension.
/// The returned extension includes its dot.
fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(dot) if dot > 0 && name.len() - dot <= MAX_EXTENSION_BYTES + 1 => name.split_at(dot),
        Some(_) | None => (name, ""),
    }
}

/// Join `stem`, `suffix` and `extension`, cutting only the stem (on a char
/// boundary) so the whole name fits in [`MAX_BYTES`]. The suffix must survive
/// the cut, or every collision candidate of a long name would be the same.
fn fit(stem: &str, suffix: &str, extension: &str) -> String {
    let budget = MAX_BYTES.saturating_sub(suffix.len() + extension.len());
    let mut end = stem.len().min(budget);
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{suffix}{extension}", &stem[..end])
}

#[cfg(test)]
mod tests {
    use super::{MAX_BYTES, candidates, sanitize};

    #[test]
    fn keeps_only_the_last_component() {
        for (raw, expected) in [
            ("../../etc/passwd", "passwd"),
            ("..\\..\\win.ini", "win.ini"),
            ("C:\\Users\\me\\a.txt", "a.txt"),
            ("/abs/path/b.jpg", "b.jpg"),
            ("dir/", "upload"),
        ] {
            assert_eq!(sanitize(raw), expected, "{raw}");
        }
    }

    #[test]
    fn never_yields_a_dot_name() {
        for raw in ["", ".", "..", "...", "  ", "\u{0}", "a/.."] {
            assert_eq!(sanitize(raw), "upload", "{raw:?}");
        }
        assert_eq!(sanitize(".bashrc"), "bashrc");
        assert_eq!(sanitize("..hidden.txt"), "hidden.txt");
        assert_eq!(sanitize("name. . "), "name");
    }

    #[test]
    fn drops_control_chars_and_replaces_reserved_punctuation() {
        assert_eq!(sanitize("a\u{0}b\nc\u{7f}.txt"), "abc.txt");
        assert_eq!(sanitize("what?<is>:this|\"*.png"), "what__is__this___.png");
    }

    #[test]
    fn prefixes_windows_reserved_stems() {
        assert_eq!(sanitize("CON"), "_CON");
        assert_eq!(sanitize("nul.txt"), "_nul.txt");
        assert_eq!(sanitize("console.txt"), "console.txt");
    }

    #[test]
    fn keeps_extension_and_case() {
        assert_eq!(sanitize("IMG_0001.HEIC"), "IMG_0001.HEIC");
        assert_eq!(sanitize("photo café.jpg"), "photo café.jpg");
    }

    #[test]
    fn caps_length_on_a_char_boundary_and_keeps_the_extension() {
        let long = format!("{}.jpg", "é".repeat(300));
        let name = sanitize(&long);
        assert!(name.len() <= MAX_BYTES, "{}", name.len());
        let stem = name.strip_suffix(".jpg").expect("extension kept");
        assert!(stem.chars().all(|ch| ch == 'é'));
    }

    #[test]
    fn candidates_suffix_before_the_extension() {
        let names: Vec<_> = candidates("IMG_1.jpg").take(3).collect();
        assert_eq!(names, ["IMG_1.jpg", "IMG_1-2.jpg", "IMG_1-3.jpg"]);
        let bare: Vec<_> = candidates("README").take(2).collect();
        assert_eq!(bare, ["README", "README-2"]);
        let nested: Vec<_> = candidates("archive.tar.gz").nth(1).into_iter().collect();
        assert_eq!(nested, ["archive.tar-2.gz"]);
    }

    #[test]
    fn candidates_stay_within_the_cap() {
        let long = sanitize(&format!("{}.jpg", "a".repeat(300)));
        let second = candidates(&long).nth(1).unwrap();
        assert!(second.len() <= MAX_BYTES);
        assert!(second.ends_with("-2.jpg"));
    }
}
