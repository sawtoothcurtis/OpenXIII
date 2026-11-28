//! In-place quicksorts that give equal elements the same final order as the 2003 engine.
//!
//! A general-purpose sort (`slice::sort_by`, `sort_unstable_by`) puts the items in the same
//! *sorted* order, but elements that compare equal can end up in another order. The engine sorts
//! line-check hits (equal Time) and GUI list items (equal text) whose ties matter for the rest of
//! the game, so these two functions reproduce its tie order. Both are written for OpenXIII: an
//! explicit work list instead of recursion, a selection pass on short ranges, the smaller side
//! handled first.
//!
//! `cmp(a, b)` returns a negative number, zero or a positive number like a C comparator.

/// Ranges of at most this many elements are finished by [`select_to_end`].
const SHORT_RANGE: isize = 8;

/// Finishes `v[first..=last]`: the largest element (the first one met among equals) goes to the
/// end, then the same on the range one shorter, and so on.
fn select_to_end<T>(v: &mut [T], first: usize, mut last: usize, cmp: &mut impl FnMut(&T, &T) -> i32) {
    while last > first {
        let mut largest = first;
        for k in first + 1..=last {
            if cmp(&v[k], &v[largest]) > 0 {
                largest = k;
            }
        }
        v.swap(largest, last);
        last -= 1;
    }
}

/// Common driver: `split(v, first, last)` partitions a range of more than [`SHORT_RANGE`]
/// elements and returns `(left_end, right_start)`, the two sides still to sort being
/// `first..=left_end` and `right_start..=last`. The larger side waits on the work list.
fn quicksort_with<T>(v: &mut [T], cmp: &mut impl FnMut(&T, &T) -> i32, mut split: impl FnMut(&mut [T], isize, isize, &mut dyn FnMut(&T, &T) -> i32) -> (isize, isize)) {
    if v.len() < 2 {
        return;
    }
    let mut waiting: Vec<(isize, isize)> = vec![(0, v.len() as isize - 1)];
    while let Some((mut first, mut last)) = waiting.pop() {
        loop {
            if last - first < SHORT_RANGE {
                select_to_end(v, first as usize, last as usize, cmp);
                break;
            }
            let (left_end, right_start) = split(v, first, last, cmp);
            let left = (first, left_end);
            let right = (right_start, last);
            let (later, now) = if left_end - first >= last - right_start { (left, right) } else { (right, left) };
            if later.0 < later.1 {
                waiting.push(later);
            }
            if now.0 >= now.1 {
                break;
            }
            (first, last) = now;
        }
    }
}

/// The order of the C runtime's `qsort` used by the engine (line-check hits): the pivot is the
/// middle of the range after ordering the first, middle and last elements, it may move during the
/// partition, and the elements equal to it next to the split are left out of both sides.
pub(crate) fn sort_like_runtime_qsort<T>(v: &mut [T], mut cmp: impl FnMut(&T, &T) -> i32) {
    quicksort_with(v, &mut cmp, |v, first, last, cmp| {
        let at = |i: isize| i as usize;
        let mut pivot = first + (last - first + 1) / 2;
        for (x, y) in [(first, pivot), (first, last), (pivot, last)] {
            if cmp(&v[at(x)], &v[at(y)]) > 0 {
                v.swap(at(x), at(y));
            }
        }
        let (mut up, mut down) = (first, last);
        loop {
            // `up` moves right over the elements not above the pivot; once it has reached the
            // pivot's slot it goes on up to the end of the range.
            if up < pivot {
                loop {
                    up += 1;
                    if up >= pivot || cmp(&v[at(up)], &v[at(pivot)]) > 0 {
                        break;
                    }
                }
            }
            if up >= pivot {
                loop {
                    up += 1;
                    if up > last || cmp(&v[at(up)], &v[at(pivot)]) > 0 {
                        break;
                    }
                }
            }
            loop {
                down -= 1;
                if down <= pivot || cmp(&v[at(down)], &v[at(pivot)]) <= 0 {
                    break;
                }
            }
            if down < up {
                break;
            }
            v.swap(at(up), at(down));
            if pivot == down {
                pivot = up;
            }
        }
        // Elements equal to the pivot just left of the split need no more sorting.
        down += 1;
        if pivot < down {
            loop {
                down -= 1;
                if down <= pivot || cmp(&v[at(down)], &v[at(pivot)]) != 0 {
                    break;
                }
            }
        }
        if pivot >= down {
            loop {
                down -= 1;
                if down <= first || cmp(&v[at(down)], &v[at(pivot)]) != 0 {
                    break;
                }
            }
        }
        (down, up)
    });
}

/// The order of the engine's own template sort (GUI lists): the middle element is moved to the
/// front as the pivot, both cursors stop on elements equal to it, and the pivot then goes between
/// the two sides.
pub(crate) fn sort_like_engine_template<T>(v: &mut [T], mut cmp: impl FnMut(&T, &T) -> i32) {
    quicksort_with(v, &mut cmp, |v, first, last, cmp| {
        let at = |i: isize| i as usize;
        v.swap(at(first + (last - first + 1) / 2), at(first));
        let (mut up, mut down) = (first, last + 1);
        loop {
            loop {
                up += 1;
                if up > last || cmp(&v[at(up)], &v[at(first)]) > 0 {
                    break;
                }
            }
            loop {
                down -= 1;
                if down <= first || cmp(&v[at(down)], &v[at(first)]) < 0 {
                    break;
                }
            }
            if up > down {
                break;
            }
            v.swap(at(up), at(down));
        }
        v.swap(at(first), at(down));
        (down - 1, up)
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_order_of_ties() {
        // Pairs (key, rank): on a short range the first largest goes to the end.
        let mut v = [(1, 0), (0, 1), (1, 2), (0, 3)];
        sort_like_runtime_qsort(&mut v, |a, b| a.0.cmp(&b.0) as i32);
        assert_eq!(v, [(0, 1), (0, 3), (1, 2), (1, 0)]);
    }

    #[test]
    fn both_sort() {
        for n in [0usize, 1, 2, 8, 9, 10, 31, 200, 1000] {
            let base: Vec<i32> = (0..n as i32).map(|i| (i * 7919) % 101).collect();
            let mut want = base.clone();
            want.sort();
            let mut a = base.clone();
            sort_like_runtime_qsort(&mut a, |x, y| x.cmp(y) as i32);
            assert_eq!(a, want, "runtime n={n}");
            let mut b = base;
            sort_like_engine_template(&mut b, |x, y| x.cmp(y) as i32);
            assert_eq!(b, want, "template n={n}");
        }
    }
}

