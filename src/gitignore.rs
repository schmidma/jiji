use std::{collections::BTreeSet, fs};

use camino::Utf8Path;
use color_eyre::{
    eyre::{bail, Context as _},
    Result,
};

use crate::{index::Index, JijiRepository};

const WORKSPACE_GITIGNORE: &str = "/cache/\n/.lock\n/config.local.toml\n";
const BEGIN_MARKER: &str = "# BEGIN Jiji tracked content";
const END_MARKER: &str = "# END Jiji tracked content";

impl JijiRepository {
    pub(crate) fn ensure_workspace_gitignore(&self) -> Result<()> {
        let path = self.workspace_root().join(".gitignore");
        fs::write(&path, WORKSPACE_GITIGNORE)
            .wrap_err_with(|| format!("failed to write workspace gitignore at {path}"))?;
        Ok(())
    }

    pub(crate) fn refresh_gitignore_for_base(&self, index: &Index, base: &Utf8Path) -> Result<()> {
        let rules = gitignore_rules_for_base(index, base);
        let gitignore_path = self.root.join(base).join(".gitignore");
        let existing = match fs::read_to_string(&gitignore_path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(error)
                    .wrap_err_with(|| format!("failed to read gitignore at {gitignore_path}"));
            }
        };

        let rewritten = rewrite_managed_block(&existing, rules)
            .wrap_err_with(|| format!("failed to update gitignore at {gitignore_path}"))?;

        if rewritten.is_empty() {
            if gitignore_path.exists() {
                fs::remove_file(&gitignore_path).wrap_err_with(|| {
                    format!("failed to remove empty gitignore at {gitignore_path}")
                })?;
            }
            return Ok(());
        }

        fs::write(&gitignore_path, rewritten)
            .wrap_err_with(|| format!("failed to write gitignore at {gitignore_path}"))?;
        Ok(())
    }
}

fn gitignore_rules_for_base(index: &Index, base: &Utf8Path) -> Vec<String> {
    let mut rules = Vec::new();
    for node in index.iter_nodes().filter(|node| node.base == base) {
        for file in &node.files {
            rules.push(format!("/{}", escape_gitignore_pattern(&file.path)));
        }
        for directory in &node.directories {
            rules.push(format!("/{}/", escape_gitignore_pattern(&directory.path)));
        }
    }
    rules
}

fn escape_gitignore_pattern(path: &Utf8Path) -> String {
    let mut escaped = String::new();
    let mut characters = path.as_str().chars().peekable();
    while let Some(character) = characters.next() {
        if matches!(character, '\\' | '[' | ']' | '*' | '?' | '!' | '#')
            || character == ' ' && characters.peek().is_none()
        {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

pub(crate) fn rewrite_managed_block(
    existing: &str,
    rules: impl IntoIterator<Item = String>,
) -> Result<String> {
    let rules = rules.into_iter().collect::<BTreeSet<_>>();
    let block_ranges = find_managed_block_ranges(existing)?;

    let [(start, end)] = block_ranges.as_slice() else {
        return match block_ranges.as_slice() {
            [] if rules.is_empty() => Ok(existing.to_string()),
            [] => Ok(append_managed_block(
                normalize_trailing_newline(existing),
                rules,
            )),
            _ => bail!("multiple Jiji-managed gitignore blocks found"),
        };
    };

    if rules.is_empty() {
        return Ok(remove_managed_block(existing, *start, *end));
    }

    let mut output = existing[..*start].to_string();
    output.push_str(BEGIN_MARKER);
    output.push('\n');
    for rule in rules {
        output.push_str(&rule);
        output.push('\n');
    }
    output.push_str(END_MARKER);
    output.push('\n');
    output.push_str(&existing[*end..]);
    Ok(output)
}

fn append_managed_block(mut output: String, rules: BTreeSet<String>) -> String {
    if rules.is_empty() {
        return output;
    }

    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str(BEGIN_MARKER);
    output.push('\n');
    for rule in rules {
        output.push_str(&rule);
        output.push('\n');
    }
    output.push_str(END_MARKER);
    output.push('\n');
    output
}

fn remove_managed_block(existing: &str, start: usize, end: usize) -> String {
    let prefix = &existing[..start];
    let suffix = &existing[end..];
    format!("{prefix}{suffix}")
}

fn find_managed_block_ranges(existing: &str) -> Result<Vec<(usize, usize)>> {
    let mut ranges = Vec::new();
    let mut line_start = 0;
    let mut pending_start = None;

    for line in existing.split_inclusive('\n') {
        let line_end = line_start + line.len();
        let marker_line = line.strip_suffix('\n').unwrap_or(line);

        if marker_line == BEGIN_MARKER {
            if pending_start.is_some() {
                bail!("Jiji-managed gitignore block is missing its end marker");
            }
            pending_start = Some(line_start);
        } else if marker_line == END_MARKER {
            let Some(start) = pending_start.take() else {
                line_start = line_end;
                continue;
            };
            ranges.push((start, line_end));
        }

        line_start = line_end;
    }

    if pending_start.is_some() {
        bail!("Jiji-managed gitignore block is missing its end marker");
    }

    Ok(ranges)
}

fn normalize_trailing_newline(input: &str) -> String {
    let trimmed = input.trim_end_matches('\n');
    if trimmed.is_empty() {
        String::new()
    } else {
        format!("{trimmed}\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_gitignore_for_base_from_index_writes_expected_rules() -> Result<()> {
        let (repo, _tmp, _guard) = crate::test_utils::setup_repo()?;
        std::fs::create_dir_all("data/images")?;
        std::fs::write("data/model.bin", "model")?;
        std::fs::write("data/images/photo.jpg", "photo")?;
        let index = repo.add(["data/model.bin", "data/images"])?;

        repo.refresh_gitignore_for_base(&index, camino::Utf8Path::new("data"))?;

        let gitignore = std::fs::read_to_string(repo.root.join("data/.gitignore"))?;
        assert_eq!(
            gitignore,
            "# BEGIN Jiji tracked content\n/images/\n/model.bin\n# END Jiji tracked content\n"
        );

        Ok(())
    }

    #[test]
    fn refresh_gitignore_for_base_escapes_gitignore_metacharacters() -> Result<()> {
        let (repo, _tmp, _guard) = crate::test_utils::setup_repo()?;
        std::fs::create_dir_all("data")?;
        std::fs::write("data/[raw].bin", "raw")?;
        let index = repo.add(["data/[raw].bin"])?;

        repo.refresh_gitignore_for_base(&index, camino::Utf8Path::new("data"))?;

        let gitignore = std::fs::read_to_string(repo.root.join("data/.gitignore"))?;
        assert!(gitignore.contains("/\\[raw\\].bin"));

        Ok(())
    }

    #[test]
    fn refresh_gitignore_for_base_escapes_trailing_spaces() -> Result<()> {
        let (repo, _tmp, _guard) = crate::test_utils::setup_repo()?;
        std::fs::create_dir_all("data")?;
        std::fs::write("data/name ", "raw")?;
        let index = repo.add(["data/name "])?;

        repo.refresh_gitignore_for_base(&index, camino::Utf8Path::new("data"))?;

        let gitignore = std::fs::read_to_string(repo.root.join("data/.gitignore"))?;
        assert!(gitignore.contains("/name\\ "));

        Ok(())
    }

    #[test]
    fn rewrite_managed_block_appends_sorted_rules_and_preserves_user_content() -> Result<()> {
        let existing = "target/\n";
        let rewritten = rewrite_managed_block(
            existing,
            [
                "/z.bin".to_string(),
                "/a.bin".to_string(),
                "/a.bin".to_string(),
            ],
        )?;

        assert_eq!(
            rewritten,
            "target/\n\n# BEGIN Jiji tracked content\n/a.bin\n/z.bin\n# END Jiji tracked content\n"
        );

        Ok(())
    }

    #[test]
    fn rewrite_managed_block_replaces_existing_block() -> Result<()> {
        let existing = "target/\n\n# BEGIN Jiji tracked content\n/old.bin\n# END Jiji tracked content\nnotes\n";

        let rewritten = rewrite_managed_block(existing, ["/new.bin".to_string()])?;

        assert_eq!(
            rewritten,
            "target/\n\n# BEGIN Jiji tracked content\n/new.bin\n# END Jiji tracked content\nnotes\n"
        );

        Ok(())
    }

    #[test]
    fn rewrite_managed_block_removes_block_when_rules_are_empty() -> Result<()> {
        let existing =
            "target/\n\n# BEGIN Jiji tracked content\n/old.bin\n# END Jiji tracked content\n";

        let rewritten = rewrite_managed_block(existing, Vec::<String>::new())?;

        assert_eq!(rewritten, "target/\n\n");

        Ok(())
    }

    #[test]
    fn rewrite_managed_block_preserves_user_content_when_no_block_and_no_rules() -> Result<()> {
        let rewritten = rewrite_managed_block("target/\n\n", Vec::<String>::new())?;

        assert_eq!(rewritten, "target/\n\n");

        Ok(())
    }

    #[test]
    fn rewrite_managed_block_ignores_marker_text_inside_user_lines() -> Result<()> {
        let existing = "user # BEGIN Jiji tracked content\n# END Jiji tracked content user\n";

        let rewritten = rewrite_managed_block(existing, Vec::<String>::new())?;

        assert_eq!(rewritten, existing);

        Ok(())
    }

    #[test]
    fn rewrite_managed_block_removes_block_without_trimming_user_content_after_it() -> Result<()> {
        let existing =
            "before\n# BEGIN Jiji tracked content\n/old.bin\n# END Jiji tracked content\nafter\n\n";

        let rewritten = rewrite_managed_block(existing, Vec::<String>::new())?;

        assert_eq!(rewritten, "before\nafter\n\n");

        Ok(())
    }

    #[test]
    fn rewrite_managed_block_errors_on_multiple_blocks() {
        let existing = "# BEGIN Jiji tracked content\n/a\n# END Jiji tracked content\n# BEGIN Jiji tracked content\n/b\n# END Jiji tracked content\n";

        let error = rewrite_managed_block(existing, ["/new.bin".to_string()])
            .unwrap_err()
            .to_string();

        assert!(error.contains("multiple Jiji-managed gitignore blocks"));
    }
}
