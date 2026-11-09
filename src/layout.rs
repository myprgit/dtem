//! Binary split tree (like kitty's "splits" layout).
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.w && y >= self.y && y < self.y + self.h
    }
}

/// `vertical == true` means children are side by side (a vertical divider), like kitty's vsplit.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Node {
    Leaf(u64),
    Split { vertical: bool, ratio: f32, a: Box<Node>, b: Box<Node> },
}

pub const BORDER: f32 = 1.0;

impl Node {
    pub fn rects(&self, r: Rect, border: f32, out: &mut Vec<(u64, Rect)>) {
        match self {
            Node::Leaf(id) => out.push((*id, r)),
            Node::Split { vertical, ratio, a, b } => {
                if *vertical {
                    let wa = ((r.w - border) * ratio).round();
                    a.rects(Rect { w: wa, ..r }, border, out);
                    b.rects(Rect { x: r.x + wa + border, w: r.w - wa - border, ..r }, border, out);
                } else {
                    let ha = ((r.h - border) * ratio).round();
                    a.rects(Rect { h: ha, ..r }, border, out);
                    b.rects(Rect { y: r.y + ha + border, h: r.h - ha - border, ..r }, border, out);
                }
            }
        }
    }

    pub fn leaves(&self, out: &mut Vec<u64>) {
        match self {
            Node::Leaf(id) => out.push(*id),
            Node::Split { a, b, .. } => {
                a.leaves(out);
                b.leaves(out);
            }
        }
    }

    pub fn leaf_list(&self) -> Vec<u64> {
        let mut v = Vec::new();
        self.leaves(&mut v);
        v
    }

    /// Split the leaf `target`, putting `new` after it.
    pub fn split(&mut self, target: u64, new: u64, vertical: bool) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                *self = Node::Split {
                    vertical,
                    ratio: 0.5,
                    a: Box::new(Node::Leaf(target)),
                    b: Box::new(Node::Leaf(new)),
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split { a, b, .. } => a.split(target, new, vertical) || b.split(target, new, vertical),
        }
    }

    /// Remove a leaf. Returns None if the tree becomes empty.
    pub fn remove(self, target: u64) -> Option<Node> {
        match self {
            Node::Leaf(id) if id == target => None,
            Node::Leaf(_) => Some(self),
            Node::Split { vertical, ratio, a, b } => match (a.remove(target), b.remove(target)) {
                (Some(a), Some(b)) => Some(Node::Split { vertical, ratio, a: Box::new(a), b: Box::new(b) }),
                (Some(n), None) | (None, Some(n)) => Some(n),
                (None, None) => None,
            },
        }
    }

    fn contains(&self, target: u64) -> bool {
        match self {
            Node::Leaf(id) => *id == target,
            Node::Split { a, b, .. } => a.contains(target) || b.contains(target),
        }
    }

    /// Move the divider of the closest ancestor split with given orientation.
    /// `delta` > 0 moves the divider right/down.
    pub fn resize(&mut self, target: u64, vertical_split: bool, delta: f32) -> bool {
        match self {
            Node::Leaf(_) => false,
            Node::Split { vertical, ratio, a, b } => {
                let in_a = a.contains(target);
                let in_b = !in_a && b.contains(target);
                if !in_a && !in_b {
                    return false;
                }
                let child = if in_a { a } else { b };
                if child.resize(target, vertical_split, delta) {
                    return true;
                }
                if *vertical == vertical_split {
                    *ratio = (*ratio + delta).clamp(0.1, 0.9);
                    return true;
                }
                false
            }
        }
    }

    /// Flip orientation of the split that directly contains `target`.
    pub fn rotate(&mut self, target: u64) -> bool {
        match self {
            Node::Leaf(_) => false,
            Node::Split { vertical, a, b, .. } => {
                let direct = matches!(**a, Node::Leaf(id) if id == target) || matches!(**b, Node::Leaf(id) if id == target);
                if direct {
                    *vertical = !*vertical;
                    return true;
                }
                a.rotate(target) || b.rotate(target)
            }
        }
    }

    /// Swap two leaf ids (used to move a window forward/backward).
    pub fn swap(&mut self, x: u64, y: u64) {
        match self {
            Node::Leaf(id) => {
                if *id == x {
                    *id = y
                } else if *id == y {
                    *id = x
                }
            }
            Node::Split { a, b, .. } => {
                a.swap(x, y);
                b.swap(x, y);
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

/// Find the neighbouring pane in a direction using geometry.
pub fn neighbor(rects: &[(u64, Rect)], from: u64, dir: Dir) -> Option<u64> {
    let cur = rects.iter().find(|(id, _)| *id == from)?.1;
    let (cx, cy) = (cur.x + cur.w / 2.0, cur.y + cur.h / 2.0);
    rects
        .iter()
        .filter(|(id, _)| *id != from)
        .filter(|(_, r)| match dir {
            Dir::Left => r.x + r.w <= cur.x + 0.5 && r.y < cur.y + cur.h && r.y + r.h > cur.y,
            Dir::Right => r.x >= cur.x + cur.w - 0.5 && r.y < cur.y + cur.h && r.y + r.h > cur.y,
            Dir::Up => r.y + r.h <= cur.y + 0.5 && r.x < cur.x + cur.w && r.x + r.w > cur.x,
            Dir::Down => r.y >= cur.y + cur.h - 0.5 && r.x < cur.x + cur.w && r.x + r.w > cur.x,
        })
        .min_by(|(_, a), (_, b)| {
            let da = (a.x + a.w / 2.0 - cx).abs() + (a.y + a.h / 2.0 - cy).abs();
            let db = (b.x + b.w / 2.0 - cx).abs() + (b.y + b.h / 2.0 - cy).abs();
            da.partial_cmp(&db).unwrap()
        })
        .map(|(id, _)| *id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_remove_neighbor() {
        let mut n = Node::Leaf(1);
        assert!(n.split(1, 2, true)); // 1 | 2
        assert!(n.split(2, 3, false)); // 1 | (2 / 3)
        assert_eq!(n.leaf_list(), vec![1, 2, 3]);
        let mut r = Vec::new();
        n.rects(Rect { x: 0.0, y: 0.0, w: 101.0, h: 101.0 }, 1.0, &mut r);
        assert_eq!(neighbor(&r, 1, Dir::Right), Some(2));
        assert_eq!(neighbor(&r, 3, Dir::Up), Some(2));
        assert_eq!(neighbor(&r, 3, Dir::Left), Some(1));
        assert!(n.resize(3, true, 0.1));
        let n = n.remove(2).unwrap();
        assert_eq!(n.leaf_list(), vec![1, 3]);
        assert!(n.remove(1).unwrap().remove(3).is_none());
    }
}
