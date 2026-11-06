//! The release's own signature on a download, checked before an update is installed (Windows'
//! installer; the release signs every file).
//!
//! GitHub's SHA-256, in the feed, says a file is the one uploaded, not who made it. So the release
//! workflow signs every file with one Ed25519 key only its signing job can use (a secret of the
//! repository's `release` environment), and asides is built with the public half
//! (`packaging/release-signing.pub`). The signature's trusted comment, signed too, names the file
//! (`file:asides-1.1.0-windows-x64-setup.exe`): version, system and architecture, so an older file,
//! or another system's, can't pass for the one offered. As helio checks its own.
//!
//! The format is minisign's, so anyone can check a download by hand (`minisign -Vm FILE -p
//! release-signing.pub`). A public key is base64 of `Ed`, an 8-byte key id and the 32-byte key. A
//! signature file is four lines: an untrusted comment; base64 of `ED`, the key id and an Ed25519
//! signature of the file's BLAKE2b-512; `trusted comment: ` and the comment; base64 of an Ed25519
//! signature of the first signature followed by the comment. The older `Ed` form, which signs the
//! file itself, is refused: minisign hasn't made it by default for years, and the release never
//! does.
#![cfg_attr(not(windows), allow(dead_code))]

use std::io::Read;
use std::path::Path;

use base64::Engine;
use blake2::{Blake2b512, Digest};
use ed25519_dalek::{Signature, VerifyingKey};

/// The longest signature file taken: minisign's, with a trusted comment naming one file, is under
/// 400 bytes.
pub const MAX_SIGNATURE: u64 = 4096;
/// What starts the first line of a minisign file, which isn't signed.
const UNTRUSTED: &str = "untrusted comment:";
/// What starts the third line of a signature file; the rest is signed.
const TRUSTED: &str = "trusted comment: ";

/// A key the release signs its files with.
#[derive(Debug)]
pub struct ReleaseKey {
    id: [u8; 8],
    key: VerifyingKey,
}

impl ReleaseKey {
    /// From minisign's text form: a public key file, or its key line alone.
    pub fn parse(text: &str) -> Result<ReleaseKey, String> {
        let bad = || "this build's release key isn't a minisign public key".to_string();
        let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with(UNTRUSTED));
        let (Some(line), None) = (lines.next(), lines.next()) else { return Err(bad()) };
        let bytes = decode(line).ok_or_else(bad)?;
        let (b"Ed", id, key) = split::<2, 8, 32>(&bytes).ok_or_else(bad)? else { return Err(bad()) };
        let key = VerifyingKey::from_bytes(key).map_err(|_| bad())?;
        Ok(ReleaseKey { id: *id, key })
    }
}

/// The release file that holds `file`'s signature.
pub fn signature_name(file: &str) -> String {
    format!("{file}.minisig")
}

/// Whether `signature`, the text of a minisign signature file, is `key`'s signature of the file at
/// `path`, made for a file named `name`.
pub fn check(key: &ReleaseKey, signature: &str, path: &Path, name: &str) -> Result<(), String> {
    let not_ours = || format!("{name} isn't signed with asides' release key");
    let malformed = || format!("{name}'s signature isn't a minisign signature");
    let mut lines = signature.lines();
    let (Some(untrusted), Some(signed), Some(trusted), Some(global)) =
        (lines.next(), lines.next(), lines.next(), lines.next())
    else {
        return Err(malformed());
    };
    let (Some(signed), Some(global), Some(comment)) = (decode(signed), decode(global), trusted.strip_prefix(TRUSTED))
    else {
        return Err(malformed());
    };
    if !untrusted.starts_with(UNTRUSTED) || lines.any(|l| !l.trim().is_empty()) {
        return Err(malformed());
    }
    let (alg, id, signed) = split::<2, 8, 64>(&signed).ok_or_else(not_ours)?;
    let global: &[u8; 64] = global.as_slice().try_into().map_err(|_| not_ours())?;
    if alg != b"ED" || *id != key.id {
        return Err(not_ours());
    }
    // The comment first: it's short, and says which file this is.
    let mut whole = Vec::with_capacity(64 + comment.len());
    whole.extend_from_slice(signed);
    whole.extend_from_slice(comment.as_bytes());
    key.key.verify_strict(&whole, &Signature::from_bytes(global)).map_err(|_| not_ours())?;
    if comment != format!("file:{name}") {
        return Err(format!("the signature offered with {name} was made for another file ({comment})"));
    }
    let hash = blake2b(path)?;
    key.key.verify_strict(&hash, &Signature::from_bytes(signed)).map_err(|_| not_ours())
}

/// The file's BLAKE2b-512, read in pieces (an installer is megabytes).
fn blake2b(path: &Path) -> Result<[u8; 64], String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut hash = Blake2b512::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hash.update(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(format!("{}: {e}", path.display())),
        }
    }
    Ok(hash.finalize().into())
}

fn decode(line: &str) -> Option<Vec<u8>> {
    base64::engine::general_purpose::STANDARD.decode(line.trim()).ok()
}

/// `bytes` as three arrays of the given lengths, if that's all it holds.
fn split<const A: usize, const B: usize, const C: usize>(bytes: &[u8]) -> Option<(&[u8; A], &[u8; B], &[u8; C])> {
    if bytes.len() != A + B + C {
        return None;
    }
    let (a, rest) = bytes.split_at(A);
    let (b, c) = rest.split_at(B);
    Some((a.try_into().ok()?, b.try_into().ok()?, c.try_into().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Made with minisign 0.12 (`minisign -G -W`, then `minisign -S -t
    /// 'file:asides-1.1.0-windows-x64-setup.exe'`) with a key made for this test and thrown away.
    const KEY: &str = "untrusted comment: minisign public key F2D444223230D502
RWQC1TAyIkTU8nUtcPTlJmcOri7IzGWNNw0HtVP/pB2NzhQOirqaEiU2
";
    const OTHER_KEY: &str = "RWRChWK3htxCQgMSpWRpOkJpHv1qnXAw1td2peMAOv4SXv+WCTC+TnMJ";
    const NAME: &str = "asides-1.1.0-windows-x64-setup.exe";
    const FILE: &[u8] = b"asides test installer\n";
    const SIGNATURE: &str = "untrusted comment: signature from minisign secret key
RUQC1TAyIkTU8oPyyZCIclw2kFqLmhoVDjI6FgZKyOMnOOA2GwwqMXUlYiDLIwEnSO8mhr1t3JBFWchmWazijsihkPw0HdSQZgE=
trusted comment: file:asides-1.1.0-windows-x64-setup.exe
RsF0iPhbnPKyW+pojjrttLmWpMORhHth7r9qYzwM3wZ7/zS2JDed0t7/NEh75Zi/cxIXXk7B2gacFEKm+BRXDQ==
";

    fn file(contents: &[u8]) -> tempfile::NamedTempFile {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut file, contents).unwrap();
        file
    }

    #[test]
    fn a_file_signed_with_the_release_key_passes() {
        let key = ReleaseKey::parse(KEY).unwrap();
        assert_eq!(check(&key, SIGNATURE, file(FILE).path(), NAME), Ok(()));
    }

    #[test]
    fn a_changed_file_fails() {
        let key = ReleaseKey::parse(KEY).unwrap();
        let err = check(&key, SIGNATURE, file(b"asides test installer!\n").path(), NAME).unwrap_err();
        assert!(err.contains("isn't signed with asides' release key"), "{err}");
    }

    #[test]
    fn another_key_fails() {
        let key = ReleaseKey::parse(OTHER_KEY).unwrap();
        let err = check(&key, SIGNATURE, file(FILE).path(), NAME).unwrap_err();
        assert!(err.contains("isn't signed with asides' release key"), "{err}");
    }

    #[test]
    fn a_signature_made_for_another_file_fails() {
        // A real signature of a real release file, offered for a newer one: its comment names the
        // file it was made for.
        let key = ReleaseKey::parse(KEY).unwrap();
        let err = check(&key, SIGNATURE, file(FILE).path(), "asides-1.2.0-windows-x64-setup.exe").unwrap_err();
        assert!(err.contains("made for another file"), "{err}");
    }

    #[test]
    fn a_changed_comment_fails() {
        let key = ReleaseKey::parse(KEY).unwrap();
        let changed = SIGNATURE.replace("1.1.0", "1.2.0");
        let err = check(&key, &changed, file(FILE).path(), "asides-1.2.0-windows-x64-setup.exe").unwrap_err();
        assert!(err.contains("isn't signed with asides' release key"), "{err}");
    }

    #[test]
    fn what_isnt_a_signature_fails() {
        let key = ReleaseKey::parse(KEY).unwrap();
        for bad in ["", "untrusted comment: x\n", "<html>Not Found</html>", &format!("{SIGNATURE}extra\n")] {
            let err = check(&key, bad, file(FILE).path(), NAME).unwrap_err();
            assert!(err.contains("isn't a minisign signature"), "{bad:?}: {err}");
        }
    }

    #[test]
    fn keys_parse_from_minisigns_files_only() {
        assert!(ReleaseKey::parse(KEY).is_ok());
        assert!(ReleaseKey::parse(OTHER_KEY).is_ok());
        for bad in ["", "RWQ=", "not a key", &format!("{KEY}{OTHER_KEY}\n")] {
            assert!(ReleaseKey::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_signature_sits_beside_its_file() {
        assert_eq!(signature_name(NAME), "asides-1.1.0-windows-x64-setup.exe.minisig");
    }
}
