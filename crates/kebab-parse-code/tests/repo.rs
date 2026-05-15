use kebab_parse_code::repo::detect_repo;
use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn init_git_repo(root: &std::path::Path) {
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(root)
            .status()
            .expect("git command failed");
    };
    run(&["init", "-q"]);
    run(&["config", "user.email", "test@test"]);
    run(&["config", "user.name", "test"]);
    fs::write(root.join("README.md"), "hi").unwrap();
    run(&["add", "README.md"]);
    run(&["commit", "-q", "-m", "init"]);
}

#[test]
fn detect_repo_returns_none_outside_git() {
    let tmp = TempDir::new().unwrap();
    let nested = tmp.path().join("a/b/c.txt");
    fs::create_dir_all(nested.parent().unwrap()).unwrap();
    fs::write(&nested, "x").unwrap();
    assert!(detect_repo(&nested).is_none());
}

#[test]
fn detect_repo_walks_up_to_git_dir() {
    let tmp = TempDir::new().unwrap();
    let repo_root = tmp.path().join("myrepo");
    fs::create_dir_all(&repo_root).unwrap();
    init_git_repo(&repo_root);
    let nested = repo_root.join("src/deep/file.rs");
    fs::create_dir_all(nested.parent().unwrap()).unwrap();
    fs::write(&nested, "x").unwrap();

    let meta = detect_repo(&nested).expect("should detect repo");
    assert_eq!(meta.name, "myrepo");
    assert!(meta.branch.is_some());
    assert!(meta.commit.is_some());
    assert_eq!(meta.commit.as_ref().unwrap().len(), 40);
}

#[test]
fn detect_repo_returns_consistent_metadata_for_paths_in_same_repo() {
    let tmp = TempDir::new().unwrap();
    let repo_root = tmp.path().join("myrepo");
    fs::create_dir_all(&repo_root).unwrap();
    init_git_repo(&repo_root);
    let f1 = repo_root.join("a.rs");
    let f2 = repo_root.join("b.rs");
    fs::write(&f1, "x").unwrap();
    fs::write(&f2, "x").unwrap();
    let m1 = detect_repo(&f1).unwrap();
    let m2 = detect_repo(&f2).unwrap();
    assert_eq!(m1.name, m2.name);
    assert_eq!(m1.commit, m2.commit);
}
