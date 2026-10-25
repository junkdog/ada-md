//! Areas and sizes in terminal cells, in place of ratatui's `Rect` and `Size` (CS-4 of the
//! clean-slate plan), with their semantics: edges saturate at `u16::MAX`, and an area never
//! reaches past it.

/// A width and a height in cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Size {
    pub width: u16,
    pub height: u16,
}

impl Size {
    pub const fn new(width: u16, height: u16) -> Size {
        Size { width, height }
    }

    /// The cells it holds.
    pub const fn area(self) -> u32 {
        self.width as u32 * self.height as u32
    }
}

/// An area of cells: its top left corner, its width and its height.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl Rect {
    /// The area at `(x, y)`, its width and height cut so that it ends by `u16::MAX`.
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Rect {
        Rect {
            x,
            y,
            width: x.saturating_add(width) - x,
            height: y.saturating_add(height) - y,
        }
    }

    /// The cells it holds.
    pub const fn area(self) -> u32 {
        self.width as u32 * self.height as u32
    }

    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub const fn left(self) -> u16 {
        self.x
    }

    /// The column after its last.
    pub const fn right(self) -> u16 {
        self.x.saturating_add(self.width)
    }

    pub const fn top(self) -> u16 {
        self.y
    }

    /// The row after its last.
    pub const fn bottom(self) -> u16 {
        self.y.saturating_add(self.height)
    }

    pub const fn as_size(self) -> Size {
        Size::new(self.width, self.height)
    }

    /// The cells in both; empty, at the larger corner, if they don't meet.
    #[must_use]
    pub fn intersection(self, other: Rect) -> Rect {
        let (x, y) = (self.x.max(other.x), self.y.max(other.y));
        let (right, bottom) = (
            self.right().min(other.right()),
            self.bottom().min(other.bottom()),
        );
        Rect {
            x,
            y,
            width: right.saturating_sub(x),
            height: bottom.saturating_sub(y),
        }
    }

    /// Whether the two share a cell.
    pub const fn intersects(self, other: Rect) -> bool {
        self.x < other.right()
            && self.right() > other.x
            && self.y < other.bottom()
            && self.bottom() > other.y
    }
}

/// A count of cells as `u16`, saturating: a width, or a count of rows or entries, taken from the
/// document can pass `u16::MAX`, where `as u16` would wrap it (F-012).
pub fn cells(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn areas_stop_at_the_last_cell() {
        let r = Rect::new(u16::MAX - 2, 10, 5, u16::MAX);
        assert_eq!(
            (r.width, r.height, r.right(), r.bottom()),
            (2, u16::MAX - 10, u16::MAX, u16::MAX)
        );
        let a = Rect::new(0, 0, 10, 4);
        assert_eq!(
            a.intersection(Rect::new(6, 2, 10, 10)),
            Rect::new(6, 2, 4, 2)
        );
        assert_eq!(
            a.intersection(Rect::new(12, 5, 3, 3)),
            Rect::new(12, 5, 0, 0)
        );
        assert!(a.intersects(Rect::new(9, 3, 1, 1)) && !a.intersects(Rect::new(10, 0, 1, 1)));
    }
}
