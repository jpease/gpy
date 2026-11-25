//! Cache keys: the path escape, its decoding, and the on-disk chunking.

use super::git_bg_tests::{cache_with_repo, clean_status};
use super::key::{
    CACHE_KEY_CHUNK_BYTES, FLAT_CACHE_KEY_MAX_BYTES, WINDOWS_CACHE_KEY_MAX_LEN, cache_file_parts,
    cache_key_path_for, find_project_root, path_to_cache_key, path_to_cache_key_for,
};
use crate::config::Config;
use crate::paths::Os;
use crate::theme::ThemeConfig;
use std::path::Path;

/// The path bytes a Unix key names; panics on a malformed key.
fn decode(key: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut bytes = key.bytes();
    while let Some(byte) = bytes.next() {
        if byte != b'_' {
            out.push(byte);
            continue;
        }
        match bytes.next() {
            Some(b'_') => out.push(b'_'),
            Some(b's') => out.push(b'/'),
            Some(b'b') => out.push(b'\\'),
            Some(b'c') => out.push(b':'),
            Some(b'w') => out.push(b' '),
            Some(b'u') => out.push(bytes.next().expect("letter after _u").to_ascii_uppercase()),
            Some(b'x') => {
                let digits = [bytes.next().expect("hex"), bytes.next().expect("hex")];
                let hex = std::str::from_utf8(&digits).expect("ASCII hex");
                out.push(u8::from_str_radix(hex, 16).expect("hex byte"));
            }
            other => panic!("malformed key token: _{other:?}"),
        }
    }
    out
}

#[test]
fn test_path_to_cache_key_escaping() {
    let path = Path::new("/home/user/my project");
    let key = path_to_cache_key(path);
    assert_eq!(
        key, "_shome_suser_smy_wproject",
        "Should escape separators with unique tokens"
    );
}

/// Paths differing only in separator/underscore placement must produce
/// distinct keys.
#[test]
fn test_path_to_cache_key_no_collision_for_separator_underscore_swaps() {
    let collision_pairs = [
        ("/a/b_c", "/a_b/c"),
        ("/x/y z", "/x y/z"),
        ("/p:q/r", "/p/q:r"),
        ("/foo_bar/baz", "/foo/bar_baz"),
    ];
    for (left, right) in collision_pairs {
        let key_left = path_to_cache_key(Path::new(left));
        let key_right = path_to_cache_key(Path::new(right));
        assert_ne!(
            key_left, key_right,
            "Paths {left} and {right} must not share a cache key"
        );
    }
}

/// The Unix escape scheme is injective: the path's bytes are recoverable
/// from the key. (The Windows arm case-folds, so it is not.)
#[test]
fn test_path_to_cache_key_is_reversible() {
    let paths: [&[u8]; 11] = [
        b"/home/user/project",
        b"/a/b_c",
        b"/a_b/c",
        b"/path with spaces/repo",
        b"/weird/__double__/under_scores",
        b"C:\\Users\\foo\\bar",
        "/unicode/café/项目/repo".as_bytes(),
        b"/odd/q?a*b<c>d\"e|f",
        b"/very/long/path/that/keeps/going/on/and/on/for/a/while/repo",
        b"/not/utf8/\xff\xfe/x\xc3",
        b"/_u_x_s/MiXeD",
    ];
    for path in paths {
        let key = path_to_cache_key_for(path, Os::Unix);
        assert_eq!(decode(&key), path, "round-trip failed for {path:?}");
    }
}

/// Pins the exact `Os::Unix` output.
#[test]
fn test_path_to_cache_key_unix_characterization() {
    let cases: [(&str, &str); 6] = [
        ("/Users/foo/project", "_s_uusers_sfoo_sproject"),
        ("/tmp/what?proj", "_stmp_swhat?proj"),
        ("/a*b<c>d\"e|f", "_sa*b<c>d\"e|f"),
        ("/home/user/my project", "_shome_suser_smy_wproject"),
        ("C:\\Users\\foo\\bar", "_uc_c_b_uusers_bfoo_bbar"),
        (
            "/a/very/deeply/nested/directory/structure/that/goes/on/and/on/and/on/and/on/and/on/for/quite/a/while/to/stress/the/escaping/logic/and/make/sure/nothing/changes/unexpectedly/when/this/function/gains/windows/hardening/as/part/of/track/demo/implementation/work",
            "_sa_svery_sdeeply_snested_sdirectory_sstructure_sthat_sgoes_son_sand_son_sand_son_sand_son_sand_son_sfor_squite_sa_swhile_sto_sstress_sthe_sescaping_slogic_sand_smake_ssure_snothing_schanges_sunexpectedly_swhen_sthis_sfunction_sgains_swindows_shardening_sas_spart_sof_strack_sdemo_simplementation_swork",
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(
            path_to_cache_key_for(input, Os::Unix),
            expected,
            "unix arm output for {input}"
        );
    }
}

/// Roots that differ only in case, Unicode normalization or bytes that are
/// not UTF-8 get keys that stay distinct on a volume folding case and
/// normalization (macOS's default), since every Unix key is lowercase ASCII.
#[test]
fn unix_keys_stay_distinct_on_a_case_and_normalization_insensitive_volume() {
    let pairs: [(&[u8], &[u8]); 4] = [
        (b"/x/App", b"/x/app"),
        ("/x/caf\u{e9}".as_bytes(), "/x/cafe\u{301}".as_bytes()),
        ("/x/\u{c9}t\u{e9}".as_bytes(), "/x/\u{e9}t\u{e9}".as_bytes()),
        (b"/x/\xff", b"/x/\xfe"),
    ];
    for (left, right) in pairs {
        let left_key = path_to_cache_key_for(left, Os::Unix);
        let right_key = path_to_cache_key_for(right, Os::Unix);
        for key in [&left_key, &right_key] {
            assert!(
                key.bytes()
                    .all(|byte| byte.is_ascii() && !byte.is_ascii_uppercase()),
                "{key} must be lowercase ASCII"
            );
        }
        assert_ne!(
            left_key, right_key,
            "{left:?} and {right:?} must not share a cache file"
        );
    }
}

/// A Unix root that is not UTF-8 keeps its bytes in the key instead of
/// collapsing to U+FFFD, so two such roots never share a file.
#[cfg(unix)]
#[test]
fn path_to_cache_key_keeps_bytes_that_are_not_utf8() {
    use std::os::unix::ffi::OsStrExt as _;

    let left = Path::new(std::ffi::OsStr::from_bytes(b"/x/\xff"));
    let right = Path::new(std::ffi::OsStr::from_bytes(b"/x/\xfe"));
    assert_eq!(path_to_cache_key(left), "_sx_s_xff");
    assert_ne!(path_to_cache_key(left), path_to_cache_key(right));
}

/// Keys up to 200 bytes keep their flat name; longer ones are split into
/// chunks of at most 50 bytes that never split a character.
#[test]
fn test_cache_key_path_unix_characterization() {
    let flat = "k".repeat(FLAT_CACHE_KEY_MAX_BYTES);
    assert_eq!(cache_key_path_for(&flat, Os::Unix), flat.as_str());
    let deep = path_to_cache_key_for(
        "/a/very/deeply/nested/directory/structure/that/goes/on/and/on/and/on/and/on/and/on/for/quite/a/while/to/stress/the/escaping/logic/and/make/sure/nothing/changes/unexpectedly/when/this/function/gains/windows/hardening/as/part/of/track/demo/implementation/work",
        Os::Unix,
    );
    assert_eq!(
        cache_key_path_for(&deep, Os::Unix),
        "_sa_svery_sdeeply_snested_sdirectory_sstructure_st/hat_sgoes_son_sand_son_sand_son_sand_son_sand_son_/sfor_squite_sa_swhile_sto_sstress_sthe_sescaping_s/logic_sand_smake_ssure_snothing_schanges_sunexpect/edly_swhen_sthis_sfunction_sgains_swindows_sharden/ing_sas_spart_sof_strack_sdemo_simplementation_swo/rk"
    );
    // 150 two-byte characters: under 200 characters, but 300 bytes.
    let wide = "é".repeat(150);
    let chunked = cache_key_path_for(&wide, Os::Unix);
    let chunks: Vec<&str> = chunked.split('/').collect();
    assert_eq!(chunks.len(), 6, "{chunked}");
    assert!(
        chunks.iter().all(|c| c.len() == CACHE_KEY_CHUNK_BYTES),
        "{chunked}"
    );
    assert_eq!(chunks.concat(), wide);
    // A three-byte character never straddles a chunk boundary.
    let cjk = "项".repeat(70);
    let cjk_chunked = cache_key_path_for(&cjk, Os::Unix);
    assert!(
        cjk_chunked
            .split('/')
            .all(|c| c.len() <= CACHE_KEY_CHUNK_BYTES),
        "{cjk_chunked}"
    );
    assert_eq!(cjk_chunked.replace('/', ""), cjk);
    // The Windows key is already capped; it is never chunked.
    let long = "k".repeat(FLAT_CACHE_KEY_MAX_BYTES + 18);
    assert_eq!(cache_key_path_for(&long, Os::Windows), long.as_str());
}

/// A root whose name is long and not ASCII still gets its git files: the key
/// is cut by bytes, so no file name passes `NAME_MAX`.
#[test]
fn write_git_caches_a_root_with_a_long_non_ascii_name() {
    let (dir, cache, _repo) = cache_with_repo();
    let repo = dir.path().join("é".repeat(100));
    std::fs::create_dir_all(repo.join(".git")).expect("git dir");
    let config = Config::default();

    let wrote = cache
        .write_git(
            &repo,
            &clean_status(),
            &config,
            &ThemeConfig::default(),
            None,
            &crate::palette::active_palette(&config),
        )
        .expect("write git");

    assert!(wrote, "the git files must be written");
    let key = path_to_cache_key(&find_project_root(&repo));
    let (parent, name) = cache_file_parts(&cache.cache_dir, &key, "git.none.ansi");
    assert!(parent.join(name).is_file(), "the reader's path must exist");
}

/// The Windows arm escapes the reserved filename characters `? * < > " |`.
#[test]
fn test_path_to_cache_key_windows_escapes_reserved_chars() {
    assert_eq!(
        path_to_cache_key_for("a?*<>\"|b", Os::Windows),
        "a_q_a_l_g_d_pb"
    );
    assert_eq!(path_to_cache_key_for(r"C:\a?b", Os::Windows), "c_c_ba_qb");
}

/// The fixture's `input` escape: `\\`, `\n`, `\r`, `\t` and `\xHH`.
fn unescape_vector(raw: &str) -> Vec<u8> {
    let mut out = Vec::new();
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.extend_from_slice(c.encode_utf8(&mut [0_u8; 4]).as_bytes());
            continue;
        }
        match chars.next() {
            Some('\\') => out.push(b'\\'),
            Some('n') => out.push(b'\n'),
            Some('r') => out.push(b'\r'),
            Some('t') => out.push(b'\t'),
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                out.push(u8::from_str_radix(&hex, 16).expect("fixture hex byte"));
            }
            other => panic!("bad fixture escape: \\{other:?}"),
        }
    }
    out
}

/// Shared vectors (`tests/fixtures/cache_key_vectors.tsv`): the Unix arm must
/// match every row and round-trip.
#[test]
fn test_path_to_cache_key_shared_vectors() {
    let fixture =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/cache_key_vectors.tsv");
    let text = std::fs::read_to_string(&fixture)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", fixture.display()));
    let mut count = 0_usize;
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split('\t');
        let (Some(raw_input), Some(expected), Some(stem)) = (cols.next(), cols.next(), cols.next())
        else {
            panic!("malformed vector row: {line:?}");
        };
        let input = unescape_vector(raw_input);
        let key = path_to_cache_key_for(&input, Os::Unix);
        assert_eq!(key, expected, "vector for {raw_input}");
        assert_eq!(decode(&key), input, "round-trip for {raw_input}");
        assert_vector_stem(raw_input, &key, stem);
        count = count.saturating_add(1);
    }
    assert!(count > 0, "no vectors read from {}", fixture.display());
}

/// The shared vector's `stem` is the relative cache-file path the
/// writer builds for `key`.
fn assert_vector_stem(raw_input: &str, key: &str, stem: &str) {
    assert_eq!(
        cache_key_path_for(key, Os::Unix),
        stem,
        "stem for {raw_input}"
    );
    #[cfg(not(windows))]
    {
        let (dir, name) = cache_file_parts(Path::new("ip"), key, "git.none.ansi");
        assert_eq!(
            dir.join(name),
            Path::new("ip").join(format!("{stem}.git.none.ansi")),
            "cache file for {raw_input}"
        );
    }
}

/// Differently-cased paths to one Windows directory collide to one key.
#[test]
fn test_path_to_cache_key_windows_case_fold_collision() {
    let left = path_to_cache_key_for(r"C:\Foo\Bar", Os::Windows);
    let right = path_to_cache_key_for(r"C:\foo\bar", Os::Windows);
    assert_eq!(
        left, right,
        "differently-cased paths to the same Windows directory must share a cache key"
    );

    // The Unix arm must NOT case-fold: distinct directories would collide.
    let unix_left = path_to_cache_key_for(r"C:\Foo\Bar", Os::Unix);
    let unix_right = path_to_cache_key_for(r"C:\foo\bar", Os::Unix);
    assert_ne!(
        unix_left, unix_right,
        "the Unix arm must keep case-distinct paths apart"
    );
}

/// Long Windows paths sharing a truncated prefix still get distinct keys.
#[test]
fn test_path_to_cache_key_windows_length_cap() {
    let long_prefix = "a".repeat(WINDOWS_CACHE_KEY_MAX_LEN + 50);
    let path_a = format!("/{long_prefix}/repo-one");
    let path_b = format!("/{long_prefix}/repo-two");

    let key_a = path_to_cache_key_for(&path_a, Os::Windows);
    let key_b = path_to_cache_key_for(&path_b, Os::Windows);

    assert!(
        key_a.chars().count() < path_a.len(),
        "an over-long Windows key must be shorter than the escaped input"
    );
    assert_ne!(
        key_a, key_b,
        "two long paths sharing a truncated prefix must not collide"
    );
    assert!(
        key_a.contains("_h") && key_b.contains("_h"),
        "an over-long Windows key must carry the hash-tail marker"
    );

    // Under the cap: no hash-tail.
    let short = path_to_cache_key_for("/Users/foo/project", Os::Windows);
    assert_eq!(short, "_susers_sfoo_sproject");
}
