use ratatui::layout::{Constraint, Direction, Layout, Rect};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SectorView {
    #[default]
    Grid,
    Spiral,
}

impl SectorView {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Grid => "Grid",
            Self::Spiral => "Spiral",
        }
    }

    #[must_use]
    pub const fn toggled(self) -> Self {
        match self {
            Self::Grid => Self::Spiral,
            Self::Spiral => Self::Grid,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutMode {
    Full,
    Compact,
    TooSmall,
}

#[derive(Debug, Clone, Copy)]
pub struct AppLayout {
    pub mode: LayoutMode,
    pub header: Rect,
    pub content: Rect,
    pub rail: Rect,
    pub footer: Rect,
}

impl AppLayout {
    #[must_use]
    pub fn calculate(area: Rect) -> Self {
        if area.width < 60 || area.height < 20 {
            return Self {
                mode: LayoutMode::TooSmall,
                header: area,
                content: Rect::default(),
                rail: Rect::default(),
                footer: Rect::default(),
            };
        }
        let header_height = if area.height == 20 { 1 } else { 2 };
        let vertical = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_height),
                Constraint::Min(8),
                Constraint::Length(1),
            ])
            .split(area);
        let rail_width = if area.width >= 120 { 15 } else { 12 };
        let horizontal = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(40), Constraint::Length(rail_width)])
            .split(vertical[1]);
        Self {
            mode: if area.width >= 120 && area.height >= 36 {
                LayoutMode::Full
            } else {
                LayoutMode::Compact
            },
            header: vertical[0],
            content: horizontal[0],
            rail: horizontal[1],
            footer: vertical[2],
        }
    }
}

#[must_use]
pub fn uniform_grid(area: Rect, columns: u16, rows: u16) -> Vec<Rect> {
    if columns == 0 || rows == 0 {
        return Vec::new();
    }
    let cell_width = area.width / columns;
    let cell_height = area.height / rows;
    if cell_width == 0 || cell_height == 0 {
        return Vec::new();
    }

    let used_width = cell_width * columns;
    let used_height = cell_height * rows;
    let origin_x = area.x + (area.width - used_width) / 2;
    let origin_y = area.y + (area.height - used_height) / 2;
    let mut result = Vec::with_capacity(usize::from(columns * rows));
    for row in 0..rows {
        for column in 0..columns {
            result.push(Rect::new(
                origin_x + column * cell_width,
                origin_y + row * cell_height,
                cell_width,
                cell_height,
            ));
        }
    }
    result
}

/// Returns row-major cell indices in center-out, clockwise spiral order.
#[must_use]
pub fn spiral_cells(columns: usize, rows: usize) -> Vec<usize> {
    let capacity = columns.saturating_mul(rows);
    if capacity == 0 {
        return Vec::new();
    }

    let mut result = Vec::with_capacity(capacity);
    let mut column = (columns.saturating_sub(1) / 2) as isize;
    let mut row = (rows.saturating_sub(1) / 2) as isize;
    result.push(row as usize * columns + column as usize);

    let directions = [(1_isize, 0_isize), (0, 1), (-1, 0), (0, -1)];
    let mut step_length = 1;
    let mut direction_index = 0;
    while result.len() < capacity {
        for _ in 0..2 {
            let (horizontal, vertical) = directions[direction_index % directions.len()];
            direction_index += 1;
            for _ in 0..step_length {
                column += horizontal;
                row += vertical;
                if column >= 0 && column < columns as isize && row >= 0 && row < rows as isize {
                    result.push(row as usize * columns + column as usize);
                    if result.len() == capacity {
                        return result;
                    }
                }
            }
        }
        step_length += 1;
    }
    result
}

#[must_use]
pub fn sector_cell_for_rank(view: SectorView, rank: usize, columns: usize, rows: usize) -> usize {
    let capacity = columns.saturating_mul(rows);
    if capacity == 0 {
        return 0;
    }
    match view {
        SectorView::Grid => rank.min(capacity - 1),
        SectorView::Spiral => spiral_cells(columns, rows)
            .get(rank)
            .copied()
            .unwrap_or(capacity - 1),
    }
}

#[must_use]
pub fn sector_rank_for_cell(
    view: SectorView,
    cell: usize,
    count: usize,
    columns: usize,
    rows: usize,
) -> Option<usize> {
    if count == 0 || cell >= columns.saturating_mul(rows) {
        return None;
    }
    match view {
        SectorView::Grid => (cell < count).then_some(cell),
        SectorView::Spiral => spiral_cells(columns, rows)
            .into_iter()
            .take(count)
            .position(|candidate| candidate == cell),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_grid_centers_equal_cells_and_leaves_remainder_as_padding() {
        let area = Rect::new(2, 3, 101, 41);
        let cells = uniform_grid(area, 3, 3);

        assert_eq!(cells.len(), 9);
        assert!(cells.iter().all(|cell| cell.width == 33));
        assert!(cells.iter().all(|cell| cell.height == 13));
        assert_eq!(cells[0], Rect::new(3, 4, 33, 13));
        assert_eq!(cells[8].right(), 102);
        assert_eq!(cells[8].bottom(), 43);
        assert_eq!(cells[0].x - area.x, area.right() - cells[8].right());
        assert_eq!(cells[0].y - area.y, area.bottom() - cells[8].bottom());
    }

    #[test]
    fn small_terminal_is_rejected() {
        assert_eq!(
            AppLayout::calculate(Rect::new(0, 0, 59, 20)).mode,
            LayoutMode::TooSmall
        );
        assert_eq!(
            AppLayout::calculate(Rect::new(0, 0, 60, 20)).mode,
            LayoutMode::Compact
        );
        let minimum = AppLayout::calculate(Rect::new(0, 0, 60, 20));
        assert_eq!(minimum.header.height, 1);
        assert_eq!(minimum.content.height, 18);
        assert_eq!(
            AppLayout::calculate(Rect::new(0, 0, 60, 21)).header.height,
            2
        );
    }

    #[test]
    fn spiral_starts_at_center_and_expands_clockwise() {
        assert_eq!(spiral_cells(3, 3), vec![4, 5, 8, 7, 6, 3, 0, 1, 2]);
        assert_eq!(spiral_cells(4, 2), vec![1, 2, 6, 5, 4, 0, 3, 7]);
    }

    #[test]
    fn spiral_rank_and_cell_mapping_round_trip() {
        for view in [SectorView::Grid, SectorView::Spiral] {
            for rank in 0..17 {
                let cell = sector_cell_for_rank(view, rank, 5, 4);
                assert_eq!(sector_rank_for_cell(view, cell, 17, 5, 4), Some(rank));
            }
        }
        let unused_spiral_cell = sector_cell_for_rank(SectorView::Spiral, 19, 5, 4);
        assert_eq!(
            sector_rank_for_cell(SectorView::Spiral, unused_spiral_cell, 17, 5, 4),
            None
        );
    }
}
