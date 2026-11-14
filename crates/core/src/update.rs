//! Looking for a newer release of the app and fetching it.
//!
//! A release holds `SHA256SUMS.txt`, which names the exe and its SHA-256,
//! and a signature of that file. The signed file alone says which version
//! is newest, what to download and what it must hash to.

use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

use minisign_verify::{PublicKey, Signature};
use sha2::{Digest, Sha256};
use thiserror::Error;

/// The file in a release that names the exe and its SHA-256.
pub const LISTING_NAME: &str = "SHA256SUMS.txt";
/// The signature of that file.
pub const SIGNATURE_NAME: &str = "SHA256SUMS.txt.minisig";

const ASSET_PREFIX: &str = "clone-bay-v";
const ASSET_SUFFIX: &str = "-windows-x64.exe";

/// Why no update was found or fetched.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UpdateError {
    #[error("couldn't reach GitHub ({0})")]
    Network(String),
    #[error("the release's signature is missing or wrong")]
    BadSignature,
    #[error("the release doesn't list a download for Windows")]
    NoAsset,
    #[error("the download doesn't match the release's checksum")]
    WrongHash,
    #[error("the download is larger than it should be")]
    TooLarge,
    #[error("the download couldn't be saved ({0})")]
    Save(String),
}

/// A release version: three plain numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    major: u32,
    minor: u32,
    patch: u32,
}

impl Version {
    /// Reads `X.Y.Z`. Anything else (a `v`, a suffix, a sign, a leading
    /// zero) is refused.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        let (Some(major), Some(minor), Some(patch), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return None;
        };
        Some(Self {
            major: number(major)?,
            minor: number(minor)?,
            patch: number(patch)?,
        })
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Digits only: `str::parse` alone would accept "+1".
fn number(text: &str) -> Option<u32> {
    let plain = !text.is_empty()
        && text.bytes().all(|byte| byte.is_ascii_digit())
        && (text.len() == 1 || !text.starts_with('0'));
    if plain {
        text.parse().ok()
    } else {
        None
    }
}

/// A release that can be downloaded: its version, the file's name in the
/// release, and the SHA-256 the file must have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Offer {
    pub version: Version,
    pub asset: String,
    pub sha256: [u8; 32],
}

/// The public keys a release may be signed with.
#[derive(Default)]
pub struct Keys(Vec<PublicKey>);

impl Keys {
    /// The keys releases are signed with: the one the release build uses,
    /// and a spare kept off GitHub. A line that isn't a key is left out,
    /// which a test catches.
    pub fn built_in() -> Self {
        let mut keys = Self::default();
        for line in include_str!("data/update_keys.txt").lines() {
            if let Some((_, key)) = line.split_once('\t') {
                keys.add(key);
            }
        }
        keys
    }

    /// Adds a minisign public key given in base64. Returns whether it was
    /// a key.
    pub fn add(&mut self, base64: &str) -> bool {
        match PublicKey::from_base64(base64.trim()) {
            Ok(key) => {
                self.0.push(key);
                true
            }
            Err(_) => false,
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Checks that `signature` is a signature of exactly `listing` by one of
/// `keys`. Only signatures made the current way (pre-hashed) are accepted.
pub fn verify_listing(listing: &str, signature: &str, keys: &Keys) -> Result<(), UpdateError> {
    let signature = Signature::decode(signature).map_err(|_| UpdateError::BadSignature)?;
    let known = keys
        .0
        .iter()
        .any(|key| key.verify(listing.as_bytes(), &signature, false).is_ok());
    if known {
        Ok(())
    } else {
        Err(UpdateError::BadSignature)
    }
}

/// Finds the one Windows download in a `SHA256SUMS.txt`. Each line is a
/// hash, two spaces and a name.
pub fn parse_listing(text: &str) -> Result<Offer, UpdateError> {
    let mut found = None;
    for line in text.lines() {
        let Some((hash, name)) = line.split_once("  ") else {
            continue;
        };
        let Some(version) = name
            .strip_prefix(ASSET_PREFIX)
            .and_then(|rest| rest.strip_suffix(ASSET_SUFFIX))
            .and_then(Version::parse)
        else {
            continue;
        };
        let sha256 = hash_bytes(hash).ok_or(UpdateError::NoAsset)?;
        if found.is_some() {
            return Err(UpdateError::NoAsset);
        }
        found = Some(Offer {
            version,
            asset: name.to_owned(),
            sha256,
        });
    }
    found.ok_or(UpdateError::NoAsset)
}

/// Larger than any real exe by a wide margin (it is under 10 MB).
pub const MAX_DOWNLOAD_BYTES: u64 = 50 * 1024 * 1024;

/// Where releases come from. GitHub in the app; a stand-in in tests.
pub trait ReleaseSource {
    /// The text of a small file in the newest published release.
    fn latest_text(&self, name: &str) -> Result<String, UpdateError>;
    /// Opens the offered file for reading.
    fn open(&self, offer: &Offer) -> Result<Box<dyn Read>, UpdateError>;
}

/// Looks at the newest release. Returns it only if its listing is signed
/// by one of `keys` and its version is higher than `running`.
pub fn check(
    source: &dyn ReleaseSource,
    keys: &Keys,
    running: Version,
) -> Result<Option<Offer>, UpdateError> {
    let listing = source.latest_text(LISTING_NAME)?;
    let signature = source.latest_text(SIGNATURE_NAME)?;
    verify_listing(&listing, &signature, keys)?;
    let offer = parse_listing(&listing)?;
    Ok((offer.version > running).then_some(offer))
}

/// Downloads the offered file to `into`, calling `progress` with the bytes
/// received so far. The file is kept only if it has the offered SHA-256;
/// on any failure nothing is left at `into`.
pub fn download(
    source: &dyn ReleaseSource,
    offer: &Offer,
    into: &Path,
    progress: &dyn Fn(u64),
) -> Result<(), UpdateError> {
    let fetched = fetch(source, offer, into, progress);
    if fetched.is_err() {
        let _ = fs::remove_file(into);
    }
    fetched
}

fn fetch(
    source: &dyn ReleaseSource,
    offer: &Offer,
    into: &Path,
    progress: &dyn Fn(u64),
) -> Result<(), UpdateError> {
    let save = |error: std::io::Error| UpdateError::Save(error.to_string());
    let mut reader = source.open(offer)?;
    let mut file = fs::File::create(into).map_err(save)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut received = 0u64;
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|error| UpdateError::Network(error.to_string()))?;
        if count == 0 {
            break;
        }
        received += count as u64;
        if received > MAX_DOWNLOAD_BYTES {
            return Err(UpdateError::TooLarge);
        }
        hasher.update(&buffer[..count]);
        file.write_all(&buffer[..count]).map_err(save)?;
        progress(received);
    }
    file.sync_all().map_err(save)?;
    let hash: [u8; 32] = hasher.finalize().into();
    if hash == offer.sha256 {
        Ok(())
    } else {
        Err(UpdateError::WrongHash)
    }
}

/// 64 lower-case hex digits as 32 bytes.
fn hash_bytes(hex: &str) -> Option<[u8; 32]> {
    let digits = hex.as_bytes();
    if digits.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    let (pairs, _) = digits.as_chunks::<2>();
    for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
        *byte = (nibble(*high)? << 4) | nibble(*low)?;
    }
    Some(bytes)
}

fn nibble(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use sha2::{Digest, Sha256};

    use super::*;

    const HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn line(name: &str) -> String {
        format!("{HASH}  {name}\n")
    }

    #[test]
    fn versions_are_three_plain_numbers() {
        assert_eq!(Version::parse("0.1.5").unwrap().to_string(), "0.1.5");
        for bad in [
            "", "1.2", "1.2.3.4", "1.2.x", "1.2.-3", "+1.2.3", "01.2.3", "1.2.3 ", "v1.2.3",
        ] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn versions_compare_as_numbers_not_text() {
        let v = |text| Version::parse(text).unwrap();
        assert!(v("0.1.10") > v("0.1.9"));
        assert!(v("0.2.0") > v("0.1.99"));
        assert!(v("1.0.0") > v("0.99.99"));
        assert_eq!(v("0.1.5"), v("0.1.5"));
    }

    #[test]
    fn the_listing_names_the_version_the_file_and_its_hash() {
        let offer = parse_listing(&line("clone-bay-v0.1.5-windows-x64.exe")).unwrap();
        assert_eq!(offer.version, Version::parse("0.1.5").unwrap());
        assert_eq!(offer.asset, "clone-bay-v0.1.5-windows-x64.exe");
        assert_eq!(offer.sha256[..2], [0x01, 0x23]);
        assert_eq!(offer.sha256[31], 0xef);
    }

    #[test]
    fn other_lines_in_the_listing_are_passed_over() {
        let text = format!(
            "{}{}",
            line("clone-bay-v0.1.5-macos.zip"),
            line("clone-bay-v0.1.5-windows-x64.exe")
        );
        assert_eq!(
            parse_listing(&text).unwrap().asset,
            "clone-bay-v0.1.5-windows-x64.exe"
        );
    }

    #[test]
    fn a_listing_that_is_not_exactly_as_expected_is_refused() {
        let no_asset = Err(UpdateError::NoAsset);
        assert_eq!(parse_listing(""), no_asset);
        assert_eq!(parse_listing(&line("something-else.exe")), no_asset);
        assert_eq!(
            parse_listing(&line("clone-bay-v1.x-windows-x64.exe")),
            no_asset
        );
        assert_eq!(
            parse_listing(&line("clone-bay-v0.1.5-windows-x64.exe.bak")),
            no_asset
        );
        assert_eq!(
            parse_listing(&line("../clone-bay-v0.1.5-windows-x64.exe")),
            no_asset
        );
        // One space, not two.
        assert_eq!(
            parse_listing(&format!("{HASH} clone-bay-v0.1.5-windows-x64.exe\n")),
            no_asset
        );
        // A hash that is short, or not lower-case hex.
        assert_eq!(
            parse_listing("abc  clone-bay-v0.1.5-windows-x64.exe\n"),
            no_asset
        );
        assert_eq!(
            parse_listing(&format!(
                "{}  clone-bay-v0.1.5-windows-x64.exe\n",
                HASH.to_uppercase()
            )),
            no_asset
        );
        // Two Windows downloads: which one is meant can't be known.
        let two = format!(
            "{}{}",
            line("clone-bay-v0.1.5-windows-x64.exe"),
            line("clone-bay-v0.1.6-windows-x64.exe")
        );
        assert_eq!(parse_listing(&two), no_asset);
    }

    /// Signs `text` with a fresh key. Returns the signature file's text and
    /// the public key in base64.
    pub(crate) fn signed(text: &str) -> (String, String) {
        let minisign::KeyPair { pk, sk } =
            minisign::KeyPair::generate_unencrypted_keypair().unwrap();
        let signature = minisign::sign(None, &sk, std::io::Cursor::new(text), None, None)
            .unwrap()
            .into_string();
        (signature, pk.to_base64())
    }

    fn keys(base64: &[&str]) -> Keys {
        let mut keys = Keys::default();
        for key in base64 {
            assert!(keys.add(key));
        }
        keys
    }

    #[test]
    fn a_listing_signed_by_a_known_key_is_accepted() {
        let (signature, key) = signed("hello\n");
        assert_eq!(
            verify_listing("hello\n", &signature, &keys(&[&key])),
            Ok(())
        );
    }

    #[test]
    fn either_built_in_key_will_do() {
        let (_, release) = signed("x");
        let (signature, backup) = signed("hello\n");
        assert_eq!(
            verify_listing("hello\n", &signature, &keys(&[&release, &backup])),
            Ok(())
        );
    }

    #[test]
    fn a_changed_listing_an_unknown_key_or_a_broken_signature_is_refused() {
        let bad = Err(UpdateError::BadSignature);
        let (signature, key) = signed("hello\n");
        let (_, stranger) = signed("hello\n");
        assert_eq!(verify_listing("hello!\n", &signature, &keys(&[&key])), bad);
        assert_eq!(
            verify_listing("hello\n", &signature, &keys(&[&stranger])),
            bad
        );
        assert_eq!(verify_listing("hello\n", &signature, &Keys::default()), bad);
        assert_eq!(
            verify_listing("hello\n", "not a signature", &keys(&[&key])),
            bad
        );
        assert_eq!(verify_listing("hello\n", "", &keys(&[&key])), bad);
    }

    /// A release held in memory.
    struct Fake {
        listing: String,
        signature: String,
        body: Vec<u8>,
        offline: bool,
    }

    impl ReleaseSource for Fake {
        fn latest_text(&self, name: &str) -> Result<String, UpdateError> {
            if self.offline {
                return Err(UpdateError::Network("offline".to_owned()));
            }
            match name {
                LISTING_NAME => Ok(self.listing.clone()),
                SIGNATURE_NAME => Ok(self.signature.clone()),
                other => panic!("asked for {other}"),
            }
        }

        fn open(&self, _offer: &Offer) -> Result<Box<dyn std::io::Read>, UpdateError> {
            Ok(Box::new(std::io::Cursor::new(self.body.clone())))
        }
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// A signed release of `version` whose exe is `body`, and its key.
    fn release(version: &str, body: &[u8]) -> (Fake, Keys) {
        let listing = format!(
            "{}  clone-bay-v{version}-windows-x64.exe\n",
            hex(&Sha256::digest(body))
        );
        let (signature, key) = signed(&listing);
        let fake = Fake {
            listing,
            signature,
            body: body.to_vec(),
            offline: false,
        };
        (fake, keys(&[&key]))
    }

    fn v(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn a_newer_release_is_offered() {
        let (fake, keys) = release("0.1.5", b"exe");
        let offer = check(&fake, &keys, v("0.1.4")).unwrap().unwrap();
        assert_eq!(offer.version, v("0.1.5"));
    }

    #[test]
    fn the_same_or_an_older_release_is_not_offered() {
        let (fake, keys) = release("0.1.5", b"exe");
        assert_eq!(check(&fake, &keys, v("0.1.5")), Ok(None));
        assert_eq!(check(&fake, &keys, v("0.2.0")), Ok(None));
    }

    #[test]
    fn a_release_signed_by_someone_else_is_not_offered() {
        let (fake, _) = release("0.1.5", b"exe");
        let (_, other_keys) = release("0.1.5", b"exe");
        assert_eq!(
            check(&fake, &other_keys, v("0.1.4")),
            Err(UpdateError::BadSignature)
        );
    }

    #[test]
    fn being_offline_is_an_error_not_an_offer() {
        let (mut fake, keys) = release("0.1.5", b"exe");
        fake.offline = true;
        assert!(matches!(
            check(&fake, &keys, v("0.1.4")),
            Err(UpdateError::Network(_))
        ));
    }

    #[test]
    fn a_download_that_matches_is_saved_and_progress_is_reported() {
        let (fake, keys) = release("0.1.5", b"the new exe");
        let offer = check(&fake, &keys, v("0.1.4")).unwrap().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let into = dir.path().join("new.exe");
        let seen = std::cell::Cell::new(0);
        download(&fake, &offer, &into, &|received| seen.set(received)).unwrap();
        assert_eq!(std::fs::read(&into).unwrap(), b"the new exe");
        assert_eq!(seen.get(), 11);
    }

    #[test]
    fn a_download_that_does_not_match_is_deleted() {
        let (mut fake, keys) = release("0.1.5", b"the new exe");
        let offer = check(&fake, &keys, v("0.1.4")).unwrap().unwrap();
        fake.body = b"something else".to_vec();
        let dir = tempfile::tempdir().unwrap();
        let into = dir.path().join("new.exe");
        assert_eq!(
            download(&fake, &offer, &into, &|_| {}),
            Err(UpdateError::WrongHash)
        );
        assert!(!into.exists());
    }

    #[test]
    fn a_download_over_the_size_limit_is_stopped_and_deleted() {
        struct Endless;
        impl ReleaseSource for Endless {
            fn latest_text(&self, _: &str) -> Result<String, UpdateError> {
                unreachable!()
            }
            fn open(&self, _: &Offer) -> Result<Box<dyn std::io::Read>, UpdateError> {
                Ok(Box::new(std::io::repeat(0)))
            }
        }
        let offer = Offer {
            version: v("0.1.5"),
            asset: "clone-bay-v0.1.5-windows-x64.exe".to_owned(),
            sha256: [0; 32],
        };
        let dir = tempfile::tempdir().unwrap();
        let into = dir.path().join("new.exe");
        assert_eq!(
            download(&Endless, &offer, &into, &|_| {}),
            Err(UpdateError::TooLarge)
        );
        assert!(!into.exists());
    }

    #[test]
    fn a_download_that_cannot_be_saved_says_so() {
        let (fake, keys) = release("0.1.5", b"exe");
        let offer = check(&fake, &keys, v("0.1.4")).unwrap().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let into = dir.path().join("no-such-folder").join("new.exe");
        assert!(matches!(
            download(&fake, &offer, &into, &|_| {}),
            Err(UpdateError::Save(_))
        ));
    }

    #[test]
    fn two_keys_are_built_in() {
        assert_eq!(Keys::built_in().len(), 2);
    }

    #[test]
    fn text_that_is_not_a_key_is_not_added() {
        let mut keys = Keys::default();
        assert!(!keys.add("not a key"));
        assert_eq!(keys.len(), 0);
    }
}
