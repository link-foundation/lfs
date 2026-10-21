//! Size-balanced trees over externally stored nodes.
//!
//! The same algorithm (Chen Qifeng's SBT, as used by linksplatform's
//! doublets) drives the trees embedded in link records and the per-link
//! trees of the optional variable-size index. Node `0` is the null node;
//! subtree sizes are plain counts, so a tree is byte-compatible with the
//! doublets `size_as_source`/`size_as_target` cells.

/// Ordering key of a node: `(primary, secondary, tie-break)`.
pub type Key = (u64, u64, u64);

/// Node storage for a tree.
pub trait Nodes {
    /// Left child of `node`.
    fn left(&self, node: u64) -> u64;
    /// Right child of `node`.
    fn right(&self, node: u64) -> u64;
    /// Number of nodes in the subtree rooted at `node`.
    fn size(&self, node: u64) -> u64;
    /// Sets the left child.
    fn set_left(&mut self, node: u64, value: u64);
    /// Sets the right child.
    fn set_right(&mut self, node: u64, value: u64);
    /// Sets the subtree size.
    fn set_size(&mut self, node: u64, value: u64);
    /// Ordering key of `node`.
    fn key(&self, node: u64) -> Key;
}

fn left<T: Nodes + ?Sized>(tree: &T, node: u64) -> u64 {
    if node == 0 {
        0
    } else {
        tree.left(node)
    }
}

fn right<T: Nodes + ?Sized>(tree: &T, node: u64) -> u64 {
    if node == 0 {
        0
    } else {
        tree.right(node)
    }
}

/// Size of a subtree (0 for the null node).
pub fn size<T: Nodes + ?Sized>(tree: &T, node: u64) -> u64 {
    if node == 0 {
        0
    } else {
        tree.size(node)
    }
}

fn fix_size<T: Nodes + ?Sized>(tree: &mut T, node: u64) {
    let value = size(tree, tree.left(node)) + size(tree, tree.right(node)) + 1;
    tree.set_size(node, value);
}

fn rotate_right<T: Nodes + ?Sized>(tree: &mut T, node: u64) -> u64 {
    let pivot = tree.left(node);
    tree.set_left(node, tree.right(pivot));
    tree.set_right(pivot, node);
    tree.set_size(pivot, tree.size(node));
    fix_size(tree, node);
    pivot
}

fn rotate_left<T: Nodes + ?Sized>(tree: &mut T, node: u64) -> u64 {
    let pivot = tree.right(node);
    tree.set_right(node, tree.left(pivot));
    tree.set_left(pivot, node);
    tree.set_size(pivot, tree.size(node));
    fix_size(tree, node);
    pivot
}

fn maintain<T: Nodes + ?Sized>(tree: &mut T, node: u64, right_grew: bool) -> u64 {
    if node == 0 {
        return 0;
    }
    let mut node = node;
    if right_grew {
        let sibling = size(tree, left(tree, node));
        if size(tree, right(tree, right(tree, node))) > sibling {
            node = rotate_left(tree, node);
        } else if size(tree, left(tree, right(tree, node))) > sibling {
            let rotated = rotate_right(tree, tree.right(node));
            tree.set_right(node, rotated);
            node = rotate_left(tree, node);
        } else {
            return node;
        }
    } else {
        let sibling = size(tree, right(tree, node));
        if size(tree, left(tree, left(tree, node))) > sibling {
            node = rotate_right(tree, node);
        } else if size(tree, right(tree, left(tree, node))) > sibling {
            let rotated = rotate_left(tree, tree.left(node));
            tree.set_left(node, rotated);
            node = rotate_right(tree, node);
        } else {
            return node;
        }
    }
    let fixed = maintain(tree, tree.left(node), false);
    tree.set_left(node, fixed);
    let fixed = maintain(tree, tree.right(node), true);
    tree.set_right(node, fixed);
    let node = maintain(tree, node, true);
    maintain(tree, node, false)
}

/// Inserts `node` (whose key must not be in the tree); returns the new root.
pub fn insert<T: Nodes + ?Sized>(tree: &mut T, root: u64, node: u64) -> u64 {
    if root == 0 {
        tree.set_left(node, 0);
        tree.set_right(node, 0);
        tree.set_size(node, 1);
        return node;
    }
    tree.set_size(root, tree.size(root) + 1);
    if tree.key(node) < tree.key(root) {
        let child = insert(tree, tree.left(root), node);
        tree.set_left(root, child);
        maintain(tree, root, false)
    } else {
        let child = insert(tree, tree.right(root), node);
        tree.set_right(root, child);
        maintain(tree, root, true)
    }
}

fn remove_min<T: Nodes + ?Sized>(tree: &mut T, node: u64) -> (u64, u64) {
    if tree.left(node) == 0 {
        return (tree.right(node), node);
    }
    tree.set_size(node, tree.size(node) - 1);
    let (child, min) = remove_min(tree, tree.left(node));
    tree.set_left(node, child);
    (node, min)
}

/// Removes the node with key `key` (it must be present); returns the new
/// root. The removed node's tree cells are cleared.
pub fn remove<T: Nodes + ?Sized>(tree: &mut T, root: u64, key: Key) -> u64 {
    debug_assert_ne!(root, 0, "key {key:?} is not in the tree");
    if root == 0 {
        return 0;
    }
    let root_key = tree.key(root);
    tree.set_size(root, tree.size(root) - 1);
    if key == root_key {
        let (l, r) = (tree.left(root), tree.right(root));
        let replacement = if l == 0 {
            r
        } else if r == 0 {
            l
        } else {
            let (new_right, min) = remove_min(tree, r);
            tree.set_left(min, l);
            tree.set_right(min, new_right);
            tree.set_size(min, tree.size(root));
            min
        };
        tree.set_left(root, 0);
        tree.set_right(root, 0);
        tree.set_size(root, 0);
        return replacement;
    }
    if key < root_key {
        let child = remove(tree, tree.left(root), key);
        tree.set_left(root, child);
    } else {
        let child = remove(tree, tree.right(root), key);
        tree.set_right(root, child);
    }
    root
}

/// Finds a node whose key's first two parts equal `(primary, secondary)`.
pub fn find<T: Nodes + ?Sized>(tree: &T, root: u64, primary: u64, secondary: u64) -> Option<u64> {
    let mut node = root;
    while node != 0 {
        let (p, s, _) = tree.key(node);
        match (primary, secondary).cmp(&(p, s)) {
            std::cmp::Ordering::Less => node = tree.left(node),
            std::cmp::Ordering::Greater => node = tree.right(node),
            std::cmp::Ordering::Equal => return Some(node),
        }
    }
    None
}

/// Number of keys strictly below `key`.
pub fn count_less<T: Nodes + ?Sized>(tree: &T, root: u64, key: Key) -> u64 {
    let mut node = root;
    let mut count = 0;
    while node != 0 {
        if tree.key(node) < key {
            count += size(tree, tree.left(node)) + 1;
            node = tree.right(node);
        } else {
            node = tree.left(node);
        }
    }
    count
}

/// Number of keys whose primary part equals `primary`.
pub fn count_primary<T: Nodes + ?Sized>(tree: &T, root: u64, primary: u64) -> u64 {
    let below = count_less(tree, root, (primary, 0, 0));
    let up_to = primary
        .checked_add(1)
        .map_or_else(|| size(tree, root), |next| count_less(tree, root, (next, 0, 0)));
    up_to - below
}

/// Visits nodes with `low <= key <= high` in order until `visit` returns
/// `false`. Returns `false` if the visit was stopped.
pub fn each_in_range<T: Nodes + ?Sized>(
    tree: &T,
    node: u64,
    low: Key,
    high: Key,
    visit: &mut dyn FnMut(u64) -> bool,
) -> bool {
    if node == 0 {
        return true;
    }
    let key = tree.key(node);
    if key > low && !each_in_range(tree, tree.left(node), low, high, visit) {
        return false;
    }
    if key >= low && key <= high && !visit(node) {
        return false;
    }
    if key < high {
        return each_in_range(tree, tree.right(node), low, high, visit);
    }
    true
}

/// Checks ordering and size invariants; returns the subtree size.
pub fn check<T: Nodes + ?Sized>(tree: &T, node: u64, low: Option<Key>, high: Option<Key>) -> Result<u64, String> {
    if node == 0 {
        return Ok(0);
    }
    let key = tree.key(node);
    if low.is_some_and(|low| key <= low) || high.is_some_and(|high| key >= high) {
        return Err(format!("node {node} with key {key:?} is out of order"));
    }
    let l = check(tree, tree.left(node), low, Some(key))?;
    let r = check(tree, tree.right(node), Some(key), high)?;
    if tree.size(node) != l + r + 1 {
        return Err(format!("node {node} has size {} but {} nodes", tree.size(node), l + r + 1));
    }
    Ok(l + r + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Vec3 {
        nodes: Vec<[u64; 4]>,
    }

    impl Nodes for Vec3 {
        fn left(&self, node: u64) -> u64 {
            self.nodes[node as usize][0]
        }
        fn right(&self, node: u64) -> u64 {
            self.nodes[node as usize][1]
        }
        fn size(&self, node: u64) -> u64 {
            self.nodes[node as usize][2]
        }
        fn set_left(&mut self, node: u64, value: u64) {
            self.nodes[node as usize][0] = value;
        }
        fn set_right(&mut self, node: u64, value: u64) {
            self.nodes[node as usize][1] = value;
        }
        fn set_size(&mut self, node: u64, value: u64) {
            self.nodes[node as usize][2] = value;
        }
        fn key(&self, node: u64) -> Key {
            (self.nodes[node as usize][3], 0, node)
        }
    }

    fn depth(tree: &Vec3, node: u64) -> u64 {
        if node == 0 {
            0
        } else {
            1 + depth(tree, tree.left(node)).max(depth(tree, tree.right(node)))
        }
    }

    #[test]
    fn insert_remove_keeps_invariants() {
        let mut tree = Vec3::default();
        tree.nodes.push([0; 4]);
        let mut root = 0;
        let mut seed = 7u64;
        for node in 1..=2000u64 {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            tree.nodes.push([0, 0, 0, (seed >> 33) % 50]);
            root = insert(&mut tree, root, node);
        }
        assert_eq!(check(&tree, root, None, None), Ok(2000));
        assert!(depth(&tree, root) <= 2 * 11 + 2);
        let in_bucket = (1..=2000).filter(|&n| tree.nodes[n as usize][3] == 7).count() as u64;
        assert_eq!(count_primary(&tree, root, 7), in_bucket);
        let mut seen = 0;
        each_in_range(&tree, root, (7, 0, 0), (7, u64::MAX, u64::MAX), &mut |_| {
            seen += 1;
            true
        });
        assert_eq!(seen, in_bucket);
        for node in (1..=2000u64).step_by(3) {
            let key = tree.key(node);
            root = remove(&mut tree, root, key);
            assert_eq!(tree.size(node), 0);
        }
        assert_eq!(check(&tree, root, None, None), Ok(2000 - 667));
        assert_eq!(count_primary(&tree, root, u64::MAX), 0);
    }

    #[test]
    fn sorted_insertions_stay_balanced() {
        let mut tree = Vec3::default();
        tree.nodes.push([0; 4]);
        let mut root = 0;
        for node in 1..=4096u64 {
            tree.nodes.push([0, 0, 0, node]);
            root = insert(&mut tree, root, node);
        }
        assert!(depth(&tree, root) <= 2 * 12 + 2);
        assert_eq!(find(&tree, root, 100, 0), Some(100));
        assert_eq!(find(&tree, root, 5000, 0), None);
    }
}
