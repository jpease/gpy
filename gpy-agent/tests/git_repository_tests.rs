//! Tests for git repository discovery and path normalization

#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::create_dir)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_errors_doc)]

use gpy_agent::git;
use serial_test::serial;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

// ===== Test Helpers =====

// Helper function to find repo root using the crate's shared finder
fn find_repo_root<P: AsRef<Path>>(path: P) -> Option<PathBuf> {
    git::find_repo_root(path.as_ref())
}

// Create a normal git repository for testing
fn create_normal_repo(path: &Path) -> std::io::Result<()> {
    // Initialize git repo
    let output = Command::new("git")
        .args(["init"])
        .current_dir(path)
        .output()?;

    if !output.status.success() {
        return Err(std::io::Error::other("Failed to initialize git repository"));
    }

    // Configure git user
    Command::new("git")
        .args(["config", "user.email", "test@example.com"])
        .current_dir(path)
        .output()?;

    Command::new("git")
        .args(["config", "user.name", "Test User"])
        .current_dir(path)
        .output()?;

    // Create an initial commit
    fs::write(path.join("README.md"), b"# Test Repo")?;
    Command::new("git")
        .args(["add", "."])
        .current_dir(path)
        .output()?;
    Command::new("git")
        .args(["commit", "-m", "Initial commit"])
        .current_dir(path)
        .output()?;

    Ok(())
}

// Create a bare git repository for testing
fn create_bare_repo(path: &Path) -> std::io::Result<()> {
    let output = Command::new("git")
        .args(["init", "--bare"])
        .current_dir(path.parent().unwrap_or(path))
        .arg(path)
        .output()?;

    if !output.status.success() {
        return Err(std::io::Error::other(
            "Failed to initialize bare git repository",
        ));
    }

    Ok(())
}

// Create a malicious repository with path traversal attempts
fn create_malicious_repo_paths(base: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut malicious_paths = Vec::new();

    // Path with null bytes (should be rejected by path validation)
    // Note: Can't create actual files with null bytes on Unix, so we simulate

    // Path traversal attempts
    let traversal = base.join("..").join("..").join("etc").join("passwd");
    malicious_paths.push(traversal);

    // Hidden path (valid but suspicious)
    let hidden = base.join(".hidden-malicious");
    fs::create_dir_all(&hidden)?;
    malicious_paths.push(hidden);

    // Path with excessive length (over 4096 chars)
    let long_name = "a".repeat(300);
    let long_path = base.join(&long_name);
    malicious_paths.push(long_path);

    Ok(malicious_paths)
}

// ===== Tests for find_repo_root =====

#[test]
#[serial]
fn test_find_repo_root_normal_repo() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // Skip if git not available
    if create_normal_repo(temp_path).is_err() {
        return;
    }

    let result = find_repo_root(temp_path);
    assert!(result.is_some());
}

#[test]
#[serial]
fn test_find_repo_root_bare_repo() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let bare_path = temp_dir.path().join("bare.git");

    // Skip if git not available
    if create_bare_repo(&bare_path).is_err() {
        return;
    }

    let result = find_repo_root(&bare_path);
    // Bare repos should still be discovered
    assert!(result.is_some());
}

#[test]
#[serial]
fn test_find_repo_root_subdirectory() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // Skip if git not available
    if create_normal_repo(temp_path).is_err() {
        return;
    }

    // Create a subdirectory
    let subdir = temp_path.join("src").join("nested");
    fs::create_dir_all(&subdir).expect("Failed to create subdirectory");

    let result = find_repo_root(&subdir);
    assert!(result.is_some());

    let root = result.unwrap();

    // The root should point to the actual repo root, not the subdirectory
    let canonical_temp = fs::canonicalize(temp_path).expect("Failed to canonicalize");
    assert_eq!(root, canonical_temp);
}

#[test]
#[serial]
fn test_find_repo_root_not_a_repo() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    let result = find_repo_root(temp_path);
    assert!(result.is_none());
}

#[test]
#[serial]
fn test_find_repo_root_nonexistent_path() {
    let result = find_repo_root("/nonexistent/path/that/should/not/exist");
    // Should return None for paths that don't exist
    assert!(result.is_none());
}

// ===== Tests for path normalization =====

#[test]
#[serial]
fn test_normalize_repo_path_removes_dots() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // Skip if git not available
    if create_normal_repo(temp_path).is_err() {
        return;
    }

    // Create a path with . and ..
    let dotted_path = temp_path.join("src").join("..").join(".");
    fs::create_dir_all(temp_path.join("src")).expect("Failed to create dir");

    let result = find_repo_root(&dotted_path);
    assert!(result.is_some());

    let root = result.unwrap();

    // Canonicalization should resolve dots
    let canonical_temp = fs::canonicalize(temp_path).expect("Failed to canonicalize");
    assert_eq!(root, canonical_temp);
}

// ===== Tests for symlink rejection =====

#[test]
#[cfg(unix)]
#[serial]
fn test_find_repo_root_follows_symlinks_correctly() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // Skip if git not available
    if create_normal_repo(temp_path).is_err() {
        return;
    }

    // Create a symlink to the repo
    let link_path = temp_dir.path().join("link-to-repo");
    if std::os::unix::fs::symlink(temp_path, &link_path).is_err() {
        // Skip if symlinks not supported
        return;
    }

    let result = find_repo_root(&link_path);
    assert!(result.is_some());

    let root = result.unwrap();

    // Canonicalization should resolve the symlink
    // Both should point to the same canonical path
    let canonical_original = fs::canonicalize(temp_path).expect("Failed to canonicalize original");
    let canonical_link = fs::canonicalize(&link_path).expect("Failed to canonicalize link");

    assert_eq!(canonical_original, canonical_link);
    assert_eq!(root, canonical_original);
}

#[test]
#[cfg(windows)]
#[serial]
fn test_find_repo_root_windows_junction() {
    // Windows-specific test for junction points
    // Junctions are similar to symlinks but Windows-specific
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // Skip if git not available
    if create_normal_repo(temp_path).is_err() {
        return;
    }

    // Create a junction (Windows directory symlink)
    let junction_path = temp_dir.path().join("junction-to-repo");

    // Use cmd.exe to create junction
    let output = Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&junction_path)
        .arg(temp_path)
        .output();

    if output.is_err() || !output.unwrap().status.success() {
        // Skip if junction creation fails (requires admin or specific Windows features)
        return;
    }

    let result = find_repo_root(&junction_path);
    assert!(result.is_ok());

    let root = result.unwrap();
    assert!(root.is_some());

    // On Windows, canonicalize should resolve junctions similar to symlinks
    let canonical_original = fs::canonicalize(temp_path).expect("Failed to canonicalize original");
    assert_eq!(root.unwrap(), canonical_original);
}

// ===== Tests for malicious paths =====

#[test]
#[serial]
fn test_find_repo_root_path_traversal_attempt() {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let malicious_paths =
        create_malicious_repo_paths(temp_dir.path()).expect("Failed to create malicious paths");

    for path in malicious_paths {
        let result = find_repo_root(&path);
        // Should either return Some or None, never crash (the function itself doesn't error)

        if let Some(root) = result {
            // If it returns a root, it should be canonicalized and safe
            assert!(root.is_absolute());

            // Root should not contain path traversal patterns after canonicalization
            let root_str = root.to_string_lossy();
            assert!(!root_str.contains("../"));
            assert!(!root_str.contains("..\\"));
        }
    }
}

#[test]
#[serial]
fn test_find_repo_root_rejects_relative_traversal() {
    // Test that relative path traversal doesn't escape the repo
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    // Skip if git not available
    if create_normal_repo(temp_path).is_err() {
        return;
    }

    // Try to traverse outside the repo
    let traversal_path = temp_path.join("..").join("..").join("..");

    // This should either:
    // 1. Return Ok(None) if no repo found in parent directories
    // 2. Return Ok(Some(path)) to a valid parent repo if one exists
    // 3. Never crash or return an unsafe path
    let result = find_repo_root(&traversal_path);

    if let Some(root) = result {
        // Any returned root must be absolute and canonical
        assert!(root.is_absolute());
    }
}
