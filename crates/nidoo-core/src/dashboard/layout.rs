//! Where widgets sit on the desktop grid. A widget with a `pos` keeps its cell, so a dashboard
//! can leave empty space between widgets; one without flows into the first free cells after
//! the previous one, the way the CSS grid places items, so dashboards saved before positions
//! existed look the same.

use std::collections::{BTreeSet, VecDeque};

use super::model::{Cell, GRID_COLUMNS, Span, Widget};

/// The cells a widget covers, in `u32` so a far row plus a height cannot overflow.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rect {
    col: u32,
    row: u32,
    w: u32,
    h: u32,
}

impl Rect {
    fn new(at: Cell, span: Span) -> Self {
        let (w, h) = span.size();
        Self {
            col: u32::from(at.col),
            row: u32::from(at.row),
            w: u32::from(w),
            h: u32::from(h),
        }
    }

    fn overlaps(&self, o: &Rect) -> bool {
        self.col < o.col + o.w
            && o.col < self.col + self.w
            && self.row < o.row + o.h
            && o.row < self.row + self.h
    }

    fn fits(&self) -> bool {
        self.col + self.w <= u32::from(GRID_COLUMNS)
    }
}

/// The first cell at or after `from` in reading order where a `span` fits on the grid without
/// covering any of `taken`.
fn first_free(taken: &[Rect], from: Cell, span: Span) -> Cell {
    let w = span.size().0.min(GRID_COLUMNS);
    let mut row = from.row;
    let mut col = from.col;
    loop {
        while col + w <= GRID_COLUMNS {
            let at = Cell { row, col };
            if !taken.iter().any(|t| t.overlaps(&Rect::new(at, span))) {
                return at;
            }
            col += 1;
        }
        row = row.saturating_add(1);
        col = 0;
    }
}

/// The cell of every widget, in list order. Positioned widgets keep their cell unless it runs
/// past the grid or covers an earlier widget; the others are placed after them, each in the
/// first free cells after the previous one. Rows no widget covers are dropped, so a gap left
/// by a removed widget does not leave an empty band.
pub fn layout(widgets: &[Widget]) -> Vec<Cell> {
    let mut cells: Vec<Option<Cell>> = vec![None; widgets.len()];
    let mut taken: Vec<Rect> = Vec::with_capacity(widgets.len());
    for (i, w) in widgets.iter().enumerate() {
        if let Some(at) = w.pos {
            let r = Rect::new(at, w.span);
            if r.fits() && !taken.iter().any(|t| t.overlaps(&r)) {
                cells[i] = Some(at);
                taken.push(r);
            }
        }
    }
    let mut cursor = Cell { row: 0, col: 0 };
    for (i, w) in widgets.iter().enumerate() {
        if cells[i].is_none() {
            let at = first_free(&taken, cursor, w.span);
            taken.push(Rect::new(at, w.span));
            cells[i] = Some(at);
            cursor = Cell {
                row: at.row,
                col: at.col + w.span.size().0,
            };
        }
    }
    let used: BTreeSet<u32> = taken.iter().flat_map(|r| r.row..r.row + r.h).collect();
    cells
        .into_iter()
        .map(|c| {
            let c = c.unwrap_or(Cell { row: 0, col: 0 });
            let empty_above = u32::from(c.row) - used.range(..u32::from(c.row)).count() as u32;
            Cell {
                row: c.row - empty_above as u16,
                col: c.col,
            }
        })
        .collect()
}

/// Checks that every positioned widget fits on the grid and that no two cover the same cell.
pub fn validate_layout(widgets: &[Widget]) -> Result<(), String> {
    let mut taken: Vec<(&str, Rect)> = Vec::new();
    for w in widgets {
        let Some(at) = w.pos else { continue };
        let r = Rect::new(at, w.span);
        if !r.fits() {
            return Err(format!(
                "widget `{}`: a {} widget at column {} runs past the {GRID_COLUMNS}-column grid",
                w.id,
                w.span.as_str(),
                at.col
            ));
        }
        if let Some((other, _)) = taken.iter().find(|(_, t)| t.overlaps(&r)) {
            return Err(format!("widgets `{other}` and `{}` overlap", w.id));
        }
        taken.push((&w.id, r));
    }
    Ok(())
}

/// Gives every widget the cell [`layout`] shows it in, so an edit starts from the screen.
pub fn pin_all(widgets: &mut [Widget]) {
    let cells = layout(widgets);
    for (w, at) in widgets.iter_mut().zip(cells) {
        w.pos = Some(at);
    }
}

/// Moves widget `index` so its top-left corner is on `to` (pulled left when it would run past
/// the grid). A widget of the same size in the way trades places with it; anything else in the
/// way moves down.
pub fn move_widget(widgets: &mut [Widget], index: usize, to: Cell) {
    pin_all(widgets);
    let Some(moved) = widgets.get(index) else {
        return;
    };
    let span = moved.span;
    let from = moved.pos.unwrap_or(to);
    let to = Cell {
        row: to.row,
        col: to.col.min(GRID_COLUMNS - span.size().0.min(GRID_COLUMNS)),
    };
    if to == from {
        return;
    }
    let target = Rect::new(to, span);
    let hits: Vec<usize> = (0..widgets.len())
        .filter(|&j| j != index && rect_of(&widgets[j]).overlaps(&target))
        .collect();
    widgets[index].pos = Some(to);
    let mut fixed = vec![index];
    if let [other] = hits[..]
        && widgets[other].span == span
        && !Rect::new(from, span).overlaps(&target)
    {
        widgets[other].pos = Some(from);
        fixed.push(other);
    }
    push_down(widgets, &fixed);
}

/// Changes widget `index` to `span`, pulled left when it would run past the grid; whatever it
/// now covers moves down.
pub fn resize_widget(widgets: &mut [Widget], index: usize, span: Span) {
    pin_all(widgets);
    let Some(w) = widgets.get_mut(index) else {
        return;
    };
    w.span = span;
    if let Some(at) = w.pos.as_mut() {
        at.col = at.col.min(GRID_COLUMNS - span.size().0.min(GRID_COLUMNS));
    }
    push_down(widgets, &[index]);
}

/// Adds `widget` in the first free cells after the last widget in reading order, so a gap
/// left on purpose earlier in the grid stays empty.
pub fn append_widget(widgets: &mut Vec<Widget>, mut widget: Widget) {
    pin_all(widgets);
    let cursor = widgets
        .iter()
        .filter_map(|w| w.pos.map(|at| (at, w.span)))
        .max_by_key(|(at, _)| *at)
        .map_or(Cell { row: 0, col: 0 }, |(at, span)| Cell {
            row: at.row,
            col: at.col + span.size().0,
        });
    let taken: Vec<Rect> = widgets.iter().map(rect_of).collect();
    widget.pos = Some(first_free(&taken, cursor, widget.span));
    widgets.push(widget);
}

fn rect_of(w: &Widget) -> Rect {
    Rect::new(w.pos.unwrap_or(Cell { row: 0, col: 0 }), w.span)
}

/// Moves down every widget that covers one of `fixed`, then whatever those cover in turn,
/// until nothing overlaps. The `fixed` widgets stay put and must not overlap each other.
fn push_down(widgets: &mut [Widget], fixed: &[usize]) {
    let mut queue: VecDeque<usize> = fixed.iter().copied().collect();
    while let Some(i) = queue.pop_front() {
        let pusher = rect_of(&widgets[i]);
        for j in 0..widgets.len() {
            if j == i || fixed.contains(&j) || !rect_of(&widgets[j]).overlaps(&pusher) {
                continue;
            }
            let mut row = pusher.row + pusher.h;
            // Below the pusher, and below any fixed widget it would land on.
            loop {
                let at = Rect {
                    row,
                    ..rect_of(&widgets[j])
                };
                match fixed
                    .iter()
                    .map(|&f| rect_of(&widgets[f]))
                    .find(|f| f.overlaps(&at))
                {
                    Some(f) => row = f.row + f.h,
                    None => break,
                }
            }
            if let Some(at) = widgets[j].pos.as_mut() {
                at.row = u16::try_from(row).unwrap_or(u16::MAX);
            }
            queue.push_back(j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dashboard::WidgetConfig;
    use crate::dashboard::WidgetKind;

    fn w(id: &str, span: Span, pos: Option<(u16, u16)>) -> Widget {
        Widget {
            id: id.into(),
            kind: WidgetKind::Sensor,
            span,
            config: WidgetConfig::Entity {
                entity_id: "sensor.t".into(),
                title: None,
            },
            pos: pos.map(|(row, col)| Cell { row, col }),
        }
    }

    fn at(row: u16, col: u16) -> Cell {
        Cell { row, col }
    }

    fn cells(ws: &[Widget]) -> Vec<(u16, u16)> {
        ws.iter()
            .map(|w| w.pos.map(|c| (c.row, c.col)).unwrap())
            .collect()
    }

    #[test]
    fn unpositioned_widgets_flow_like_the_css_grid() {
        let ws = [
            w("a", Span::X2x2, None),
            w("b", Span::X1x1, None),
            w("c", Span::X4x2, None),
            w("d", Span::X1x1, None),
        ];
        assert_eq!(layout(&ws), vec![at(0, 0), at(0, 2), at(1, 2), at(2, 0)]);
    }

    #[test]
    fn positioned_widgets_keep_their_cell_and_leave_gaps() {
        let ws = [
            w("ac", Span::X1x1, Some((0, 0))),
            w("t", Span::X1x1, Some((1, 0))),
            w("ac2", Span::X1x1, Some((0, 1))),
        ];
        assert_eq!(layout(&ws), vec![at(0, 0), at(1, 0), at(0, 1)]);
    }

    #[test]
    fn a_bad_position_falls_back_to_the_flow() {
        let ws = [
            w("a", Span::X1x1, Some((0, 0))),
            // Covers `a`.
            w("b", Span::X2x1, Some((0, 0))),
            // Runs past the grid.
            w("c", Span::X2x1, Some((0, 5))),
        ];
        assert_eq!(layout(&ws), vec![at(0, 0), at(0, 1), at(0, 3)]);
    }

    #[test]
    fn empty_rows_are_dropped() {
        let ws = [
            w("a", Span::X1x2, Some((0, 0))),
            w("b", Span::X1x1, Some((4, 3))),
            w("c", Span::X1x1, Some((7, 0))),
        ];
        assert_eq!(layout(&ws), vec![at(0, 0), at(2, 3), at(3, 0)]);
    }

    #[test]
    fn validate_refuses_overflow_and_overlap() {
        assert!(validate_layout(&[w("a", Span::X2x1, Some((0, 4)))]).is_ok());
        let err = validate_layout(&[w("a", Span::X2x1, Some((0, 5)))]).unwrap_err();
        assert!(err.contains("runs past"), "{err}");
        let err = validate_layout(&[
            w("a", Span::X2x2, Some((0, 0))),
            w("b", Span::X1x1, Some((1, 1))),
        ])
        .unwrap_err();
        assert_eq!(err, "widgets `a` and `b` overlap");
        assert!(validate_layout(&[w("a", Span::X1x1, None), w("b", Span::X1x1, None)]).is_ok());
    }

    #[test]
    fn moving_into_an_empty_cell_leaves_the_others() {
        let mut ws = vec![
            w("a", Span::X1x1, None),
            w("b", Span::X1x1, None),
            w("c", Span::X1x1, None),
        ];
        move_widget(&mut ws, 2, at(1, 0));
        assert_eq!(cells(&ws), vec![(0, 0), (0, 1), (1, 0)]);
    }

    #[test]
    fn a_widget_of_the_same_size_trades_places() {
        let mut ws = vec![w("a", Span::X1x1, None), w("b", Span::X1x1, None)];
        move_widget(&mut ws, 0, at(0, 1));
        assert_eq!(cells(&ws), vec![(0, 1), (0, 0)]);
    }

    #[test]
    fn a_different_widget_in_the_way_moves_down_and_takes_its_followers() {
        let mut ws = vec![
            w("big", Span::X2x2, Some((0, 2))),
            w("a", Span::X1x1, Some((0, 0))),
            w("under", Span::X1x1, Some((1, 0))),
        ];
        move_widget(&mut ws, 0, at(0, 0));
        assert_eq!(cells(&ws), vec![(0, 0), (2, 0), (3, 0)]);
        assert!(validate_layout(&ws).is_ok());
    }

    #[test]
    fn a_move_past_the_right_edge_is_pulled_back() {
        let mut ws = vec![w("a", Span::X2x1, None)];
        move_widget(&mut ws, 0, at(0, 5));
        assert_eq!(cells(&ws), vec![(0, 4)]);
    }

    #[test]
    fn growing_pushes_the_neighbour_below() {
        let mut ws = vec![
            w("a", Span::X1x1, None),
            w("b", Span::X1x1, None),
            w("c", Span::X1x1, Some((1, 1))),
        ];
        resize_widget(&mut ws, 0, Span::X2x2);
        assert_eq!(cells(&ws), vec![(0, 0), (2, 1), (3, 1)]);
        assert!(validate_layout(&ws).is_ok());
    }

    #[test]
    fn growing_at_the_right_edge_moves_left() {
        let mut ws = vec![w("a", Span::X1x1, Some((0, 5)))];
        resize_widget(&mut ws, 0, Span::X2x1);
        assert_eq!(cells(&ws), vec![(0, 4)]);
    }

    #[test]
    fn appending_goes_after_the_last_widget_not_into_a_gap() {
        let mut ws = vec![
            w("ac", Span::X1x1, Some((0, 0))),
            w("t", Span::X1x1, Some((1, 0))),
        ];
        append_widget(&mut ws, w("new", Span::X1x1, None));
        assert_eq!(cells(&ws), vec![(0, 0), (1, 0), (1, 1)]);
        let mut empty = vec![];
        append_widget(&mut empty, w("first", Span::X2x1, None));
        assert_eq!(cells(&empty), vec![(0, 0)]);
    }
}
