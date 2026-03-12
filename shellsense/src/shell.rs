pub fn remove_init_loader_lines(content: &str) -> String {
    let mut filtered = String::new();

    for line in content.lines() {
        let trimmed = line.trim();
        let is_loader = matches!(
            trimmed,
            r#"eval "$(shellsense init zsh)""#
                | r#"eval "$(shellsense init bash)""#
                | "shellsense init fish | source"
        );

        if !is_loader {
            filtered.push_str(line);
            filtered.push('\n');
        }
    }

    filtered
}

#[cfg(test)]
mod tests {
    use super::remove_init_loader_lines;

    #[test]
    fn removes_exact_loader_lines_only() {
        let original = "\
eval \"$(shellsense init zsh)\"
# ShellSense note
echo shellsense init zsh
shellsense init fish | source
";
        let filtered = remove_init_loader_lines(original);

        assert!(!filtered.contains("eval \"$(shellsense init zsh)\""));
        assert!(!filtered.contains("shellsense init fish | source"));
        assert!(filtered.contains("# ShellSense note"));
        assert!(filtered.contains("echo shellsense init zsh"));
    }
}
