//! Maschere alla Total Commander, stessa semantica di `src/lib/mask.ts`:
//! "inc1;inc2 inc3|exc1;exc2", senza distinzione di maiuscole, jolly `*` e `?`.

/// `*` = qualsiasi sequenza, `?` = esattamente un carattere. Backtracking sull'ultimo `*`.
fn wildcard(name: &[char], pat: &[char]) -> bool {
    let (mut n, mut p) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pat.len() && pat[p] == '*' {
            star = Some((p, n));
            p += 1;
        } else if p < pat.len() && (pat[p] == '?' || pat[p] == name[n]) {
            n += 1;
            p += 1;
        } else if let Some((sp, sn)) = star {
            p = sp + 1;
            n = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    pat[p..].iter().all(|&c| c == '*')
}

fn any_match(name: &[char], list: &str) -> Option<bool> {
    let mut seen = false;
    for pat in list.split([';', ' ']).filter(|s| !s.is_empty()) {
        seen = true;
        // `*.*` in TC combacia anche con i nomi senza estensione: equivale a `*`.
        let pat: Vec<char> = if pat == "*.*" { vec!['*'] } else { pat.to_lowercase().chars().collect() };
        if wildcard(name, &pat) {
            return Some(true);
        }
    }
    seen.then_some(false)
}

pub fn match_mask(name: &str, mask: &str) -> bool {
    let name: Vec<char> = name.to_lowercase().chars().collect();
    let (include, exclude) = mask.split_once('|').unwrap_or((mask, ""));
    any_match(&name, exclude) != Some(true) && any_match(&name, include) != Some(false)
}

#[cfg(test)]
mod tests {
    use super::match_mask;

    #[test]
    fn mask_matches_ts_semantics() {
        assert!(match_mask("a.TXT", "*.txt"));
        assert!(match_mask("a.md", "*.txt;*.md"));
        assert!(match_mask("a.md", "*.txt *.md"));
        assert!(!match_mask("a.bak.txt", "*.txt|*.bak.*")); // '|' esclude
        assert!(match_mask("ab", "a?"));
        assert!(match_mask("noext", "*.*"));
    }

    #[test]
    fn empty_mask_or_only_exclusions_includes_all() {
        assert!(match_mask("x", ""));
        assert!(!match_mask("a.bak", "|*.bak"));
        assert!(match_mask("a.txt", "|*.bak"));
    }

    #[test]
    fn literal_chars_and_lengths() {
        assert!(match_mask("a.txt", "a.txt"));
        assert!(!match_mask("abtxt", "a.txt"));
        assert!(match_mask("a(1)+[x].txt", "a(1)+[x].*"));
        assert!(!match_mask("abc", "a?"));
        assert!(match_mask("abc", "a*c"));
        assert!(match_mask("aXbYc", "a*b*c"));
        assert!(!match_mask("aXbY", "a*b*c"));
    }
}
