//! 검출 값 제외 — 체크를 해제한(또는 우클릭으로 제외한) 값은 치환하지 않는다.
//! 제외 목록은 프로그램 메모리에만 두는 원본 문자열 집합이며, 같은 문자열은 모든 파일에서 함께 제외된다.
//! 엔진에는 검사할 때 이 집합을 넘기고(`Options::excluded`), 엔진은 그 값을 아예 검출하지 않는다.
//! 그래서 같은 폴더를 다시 검사하면 이전 결과의 제외 행을 이어받아 화면에 계속 보여준다.

use crate::gui::scan::Scan;
use std::collections::{HashMap, HashSet};

/// 체크 상자 상태 = 리스트뷰 상태 이미지 번호 (0 은 상자 없음)
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Check {
    /// 제외됨 (치환 안 함)
    Off = 1,
    /// 치환함
    On = 2,
    /// 묶음 안에 둘이 섞임
    Mixed = 3,
}

/// 여러 값의 체크 상태
pub fn check<'a>(vals: impl IntoIterator<Item = &'a str>, set: &HashSet<String>) -> Check {
    let (mut on, mut off) = (false, false);
    for v in vals {
        if set.contains(v) {
            off = true;
        } else {
            on = true;
        }
    }
    match (on, off) {
        (true, true) => Check::Mixed,
        (false, true) => Check::Off,
        _ => Check::On,
    }
}

/// 체크 상자 누름: 하나라도 치환 대상이면 모두 제외, 모두 제외돼 있으면 모두 되살림
pub fn toggle(set: &mut HashSet<String>, vals: &[String]) {
    let exclude = vals.iter().any(|v| !set.contains(v));
    set_all(set, vals, exclude);
}

pub fn set_all(set: &mut HashSet<String>, vals: &[String], exclude: bool) {
    for v in vals {
        if exclude {
            set.insert(v.clone());
        } else {
            set.remove(v);
        }
    }
}

/// 지금 결과에서 제외된 검출 건수
pub fn count(scan: &Scan, set: &HashSet<String>) -> usize {
    scan.all_records().filter(|r| set.contains(&r.original)).count()
}

/// 반영하려면 다시 검사해야 하는가: 엔진이 이번 검사에 쓴 제외 목록(`applied`)과 지금 제외 상태가 다른 행이 있으면
pub fn needs_rescan(scan: &Scan, set: &HashSet<String>, applied: &HashSet<String>) -> bool {
    scan.all_records().any(|r| set.contains(&r.original) != applied.contains(&r.original))
}

/// 같은 폴더를 다시 검사한 결과에, 이전 결과에서 제외해 둔 행(엔진이 이번엔 검출하지 않은 값)을 이어 붙인다
pub fn carry_over(prev: &Scan, cur: &mut Scan, applied: &HashSet<String>) {
    if prev.root != cur.root || applied.is_empty() {
        return;
    }
    let files: HashMap<_, _> = cur.entries.iter().enumerate().map(|(i, e)| (e.path.clone(), i)).collect();
    for e in &prev.entries {
        let Some(&i) = files.get(&e.path) else { continue };
        let keep = e.records.iter().filter(|r| applied.contains(&r.original)).cloned();
        cur.entries[i].records.extend(keep);
    }
    for n in &prev.names {
        let keep: Vec<_> = n.records.iter().filter(|r| applied.contains(&r.original)).cloned().collect();
        if keep.is_empty() {
            continue;
        }
        match cur.names.iter_mut().find(|c| c.path == n.path) {
            Some(c) => c.records.extend(keep),
            None => cur.names.push(crate::ipc::NameEntry { path: n.path.clone(), is_dir: n.is_dir, records: keep }),
        }
    }
    cur.version += 1;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(v: &[&str]) -> HashSet<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn toggle_and_check() {
        let mut s = HashSet::new();
        let vals = vec!["a".to_string(), "b".to_string()];
        assert_eq!(check(["a", "b"], &s), Check::On);
        toggle(&mut s, &vals[..1]);
        assert_eq!(check(["a", "b"], &s), Check::Mixed);
        toggle(&mut s, &vals); // 섞여 있으면 모두 제외
        assert_eq!(check(["a", "b"], &s), Check::Off);
        toggle(&mut s, &vals); // 모두 제외면 모두 되살림
        assert!(s.is_empty());
    }

    #[test]
    fn same_value_everywhere_and_rescan_rule() {
        let scan = crate::win32_gui::tree::tests::sample(9); // 파일 3개 × (핸드폰 1 + "10.0.0.1" 1)
        let s = set(&["10.0.0.1"]);
        assert_eq!(count(&scan, &s), 3, "같은 값은 모든 파일에서 함께 제외");
        assert!(needs_rescan(&scan, &s, &HashSet::new()));
        assert!(!needs_rescan(&scan, &s, &s), "엔진이 이미 적용한 제외면 다시 검사하지 않음");
        assert!(!needs_rescan(&scan, &set(&["다른 폴더 값"]), &HashSet::new()), "결과에 없는 값은 상관없음");
    }

    #[test]
    fn carry_over_keeps_excluded_rows() {
        let prev = crate::win32_gui::tree::tests::sample(9);
        let mut cur = crate::win32_gui::tree::tests::sample(9);
        let applied = set(&["10.0.0.1"]);
        for e in &mut cur.entries {
            e.records.retain(|r| !applied.contains(&r.original)); // 엔진이 제외 값을 검출하지 않은 결과
        }
        assert_eq!(count(&cur, &applied), 0);
        carry_over(&prev, &mut cur, &applied);
        assert_eq!(count(&cur, &applied), 3);
        assert_eq!(cur.all_records().count(), 6);
    }
}
