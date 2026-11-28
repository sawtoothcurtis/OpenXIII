//! Sorting of actors by priority in ULevel::ServerTickClient: the Sort of UnTemplate.h, with Compare(A, B) = B.Priority − A.Priority (as
//! overflowing 32-bit integers). Non-recursive quicksort (stack of 32 ranges), pivot taken
//! in the middle, selection sort for 8 elements or fewer: it is not stable, and the order of
//! equal priorities depends on it.

/// Compare(*A, *B) of the FActorPriority*: B.Priority − A.Priority.
fn compare(a: i32, b: i32) -> i32 {
    b.wrapping_sub(a)
}

/// Sort<FActorPriority*>(First, Num): `v` sorted by decreasing priority as in the original.
pub fn tri_priorities<T>(v: &mut [T], prio: impl Fn(&T) -> i32) {
    if v.len() < 2 {
        return;
    }
    let mut pile: Vec<(usize, usize)> = vec![(0, v.len() - 1)];
    while let Some((mut min, mut max)) = pile.pop() {
        loop {
            let count = max - min + 1;
            if count <= 8 {
                // Selection: the "largest" in the sense of Compare goes to the end.
                while max > min {
                    let mut m = min;
                    for item in min + 1..=max {
                        if compare(prio(&v[item]), prio(&v[m])) > 0 {
                            m = item;
                        }
                    }
                    v.swap(m, max);
                    max -= 1;
                }
                break;
            }
            v.swap(min + count / 2, min);
            let p = prio(&v[min]);
            // Inner.Min starts at Min, Inner.Max at Max + 1 (as signed indices: Inner.Max can
            // go down to Min).
            let (mut lo, mut hi) = (min, max + 1);
            loop {
                lo += 1;
                while lo <= max && compare(prio(&v[lo]), p) <= 0 {
                    lo += 1;
                }
                hi -= 1;
                while hi > min && compare(prio(&v[hi]), p) >= 0 {
                    hi -= 1;
                }
                if lo > hi {
                    break;
                }
                v.swap(lo, hi);
            }
            v.swap(min, hi);
            // The larger half waits on the stack, we continue with the smaller one.
            if hi as isize - 1 - min as isize >= max as isize - lo as isize {
                if min + 1 < hi {
                    pile.push((min, hi - 1));
                }
                if max > lo {
                    min = lo;
                    continue;
                }
            } else {
                if max > lo {
                    pile.push((lo, max));
                }
                if min + 1 < hi {
                    max = hi - 1;
                    continue;
                }
            }
            break;
        }
    }
}

