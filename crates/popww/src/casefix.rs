use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

/// Returns the on-disk spelling of `path` if it exists only under a different letter case.
///
/// The game lowercases its paths, which Windows tolerates but a case-sensitive file system does not;
/// Wine's own case folding fails when a lowercase twin exists (`~/games` beside `~/Games`), so every
/// spelling is tried. If no spelling exists completely, the longest existing prefix wins and the rest
/// keeps its spelling, so paths of files about to be created get their directories corrected.
pub fn resolve(path: &Path) -> Option<PathBuf> {
    let raw = path.as_os_str().as_encoded_bytes();
    if raw.is_empty() || raw.starts_with(b"\\\\") || raw.starts_with(b"//") {
        return None;
    }
    if path.symlink_metadata().is_ok() {
        return None;
    }
    let components: Vec<Component> = path.components().collect();
    let mut best = Partial { real: PathBuf::new(), matched: 0 };
    let real = search(PathBuf::new(), &components, 0, &mut best).unwrap_or_else(|| {
        let mut real = best.real;
        real.extend(components[best.matched..].iter().map(|c| c.as_os_str()));
        real
    });
    (real.as_os_str() != path.as_os_str()).then_some(real)
}

struct Partial {
    real: PathBuf,
    matched: usize,
}

/// Depth-first over all spellings, exact one first; None if none exists completely.
fn search(real: PathBuf, components: &[Component], index: usize, best: &mut Partial) -> Option<PathBuf> {
    if index > best.matched {
        *best = Partial { real: real.clone(), matched: index };
    }
    let Some(component) = components.get(index) else {
        return Some(real);
    };
    let Component::Normal(name) = component else {
        return search(real.join(component.as_os_str()), components, index + 1, best);
    };
    spellings(&real, name)
        .into_iter()
        .find_map(|spelling| search(real.join(spelling), components, index + 1, best))
}

fn spellings(dir: &Path, name: &OsStr) -> Vec<OsString> {
    let mut found = Vec::new();
    if dir.join(name).symlink_metadata().is_ok() {
        found.push(name.to_os_string());
    }
    let listed = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
    let wanted = name.to_string_lossy().to_lowercase();
    let Ok(entries) = std::fs::read_dir(listed) else {
        return found;
    };
    for entry in entries.flatten() {
        let candidate = entry.file_name();
        if candidate != name && candidate.to_string_lossy().to_lowercase() == wanted {
            found.push(candidate);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game_dir(test: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("popww-casefix-{test}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let game = root.join("Games/Heroic/Prince of Persia - Warrior Within");
        std::fs::create_dir_all(game.join("Video")).unwrap();
        std::fs::write(game.join("POPData.BF"), b"").unwrap();
        std::fs::write(game.join("Video/LOADIN0.BIK"), b"").unwrap();
        root
    }

    #[test]
    fn existing_path_needs_no_fix() {
        let root = game_dir("existing");
        assert_eq!(resolve(&root.join("Games/Heroic/Prince of Persia - Warrior Within/POPData.BF")), None);
    }

    #[test]
    fn lowercased_path_gets_real_spelling() {
        let root = game_dir("lower");
        let fixed = resolve(&root.join("games/heroic/prince of persia - warrior within/video/loadin0.bik"));
        assert_eq!(fixed, Some(root.join("Games/Heroic/Prince of Persia - Warrior Within/Video/LOADIN0.BIK")));
    }

    #[test]
    fn missing_tail_keeps_its_spelling() {
        let root = game_dir("tail");
        let fixed = resolve(&root.join("games/HEROIC/New Folder/save.dat"));
        assert_eq!(fixed, Some(root.join("Games/Heroic/New Folder/save.dat")));
    }

    #[test]
    fn lowercase_twin_directory_is_backtracked() {
        let root = game_dir("twin");
        std::fs::create_dir_all(root.join("games/other")).unwrap();
        let fixed = resolve(&root.join("games/heroic/prince of persia - warrior within/popdata.bf"));
        assert_eq!(fixed, Some(root.join("Games/Heroic/Prince of Persia - Warrior Within/POPData.BF")));
    }

    #[test]
    fn unrelated_path_is_left_alone() {
        let root = game_dir("unrelated");
        assert_eq!(resolve(&root.join("nothing/here.txt")), None);
    }
}
