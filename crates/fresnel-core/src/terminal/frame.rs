//! The copy of the rows on the screen that a frame paints.
//!
//! The copy has the final colors of each cell and the modes that the keys and the pointer
//! need, so the painter reads no type of `alacritty_terminal` and holds no lock.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{TermMode, point_to_viewport};
use alacritty_terminal::vte::ansi::CursorShape as Shape;

use super::palette::{Palette, dim};
use super::screen::Screen;
use super::{GridSize, Terminal};
use crate::config::Rgb;

/// One cell with its colors after dim, inverse and the colors of the remote program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cell {
    /// A space for a blank, a hidden and a spacer cell.
    pub c: char,
    pub fg: Rgb,
    pub bg: Rgb,
    pub flags: CellFlags,
}

/// The marks of a cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CellFlags(u8);

impl CellFlags {
    pub const BOLD: Self = Self(1);
    pub const ITALIC: Self = Self(1 << 1);
    pub const UNDERLINE: Self = Self(1 << 2);
    pub const STRIKE: Self = Self(1 << 3);
    /// The first cell of a character of two columns.
    pub const WIDE: Self = Self(1 << 4);
    /// The second cell of a wide character, or the last cell of a row whose wide character
    /// went to the next row: it shows no character.
    pub const SPACER: Self = Self(1 << 5);
    /// The cell is in the selection.
    pub const SELECTED: Self = Self(1 << 6);
    /// The character has marks of zero width: [`Frame::marks`].
    pub const MARKS: Self = Self(1 << 7);

    pub fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0
    }

    fn put(&mut self, flag: Self, on: bool) {
        if on {
            self.0 |= flag.0;
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    Block,
    Underline,
    Beam,
    HollowBlock,
}

/// The cursor on the screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cursor {
    pub row: usize,
    pub column: usize,
    pub shape: CursorShape,
    /// The cursor is on a character of two columns.
    pub wide: bool,
}

/// Which events of the pointer the remote program asked for (modes 1000, 1002, 1003).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseMode {
    #[default]
    Off,
    Click,
    Drag,
    Motion,
}

/// The form of a report of the pointer (modes 1005 and 1006).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseEncoding {
    #[default]
    Normal,
    Utf8,
    Sgr,
}

/// The modes that the remote program set, for the keys, the paste and the pointer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    pub app_cursor: bool,
    pub alt_screen: bool,
    /// The wheel on the alternate screen sends arrow keys.
    pub alternate_scroll: bool,
    pub bracketed_paste: bool,
    pub focus_reports: bool,
    pub mouse: MouseMode,
    pub mouse_encoding: MouseEncoding,
}

impl Modes {
    fn of(mode: TermMode) -> Self {
        let mouse = if mode.contains(TermMode::MOUSE_MOTION) {
            MouseMode::Motion
        } else if mode.contains(TermMode::MOUSE_DRAG) {
            MouseMode::Drag
        } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
            MouseMode::Click
        } else {
            MouseMode::Off
        };
        let mouse_encoding = if mode.contains(TermMode::SGR_MOUSE) {
            MouseEncoding::Sgr
        } else if mode.contains(TermMode::UTF8_MOUSE) {
            MouseEncoding::Utf8
        } else {
            MouseEncoding::Normal
        };
        Self {
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            focus_reports: mode.contains(TermMode::FOCUS_IN_OUT),
            mouse,
            mouse_encoding,
        }
    }
}

/// The rows on the screen at one moment.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    pub size: GridSize,
    /// The cells, row after row.
    cells: Vec<Cell>,
    /// The marks of zero width of each cell that has some, by the index of the cell.
    marks: Vec<(usize, String)>,
    /// `None` when the cursor is hidden or below the screen.
    pub cursor: Option<Cursor>,
    pub modes: Modes,
    /// The lines of history above the screen: 0 shows the bottom.
    pub offset: usize,
    /// The lines of history.
    pub history: usize,
    /// The version of the screen and the palette of the copy.
    copied: Option<(u64, Palette)>,
}

impl Frame {
    /// The cells of a row of the screen. Empty for a row that the screen does not have.
    pub fn row(&self, row: usize) -> &[Cell] {
        let columns = usize::from(self.size.columns);
        self.cells.get(row * columns..(row + 1) * columns).unwrap_or_default()
    }

    /// The marks of zero width of a cell, such as an accent.
    pub fn marks(&self, row: usize, column: usize) -> Option<&str> {
        let index = row * usize::from(self.size.columns) + column;
        let found = self.marks.binary_search_by_key(&index, |(at, _)| *at).ok()?;
        self.marks.get(found).map(|(_, marks)| marks.as_str())
    }

    /// The text of a row with no space at its end: for a screen reader and for the tests.
    pub fn text(&self, row: usize) -> String {
        let mut text = String::new();
        for (column, cell) in self.row(row).iter().enumerate() {
            if !cell.flags.contains(CellFlags::SPACER) {
                text.push(cell.c);
                text.push_str(self.marks(row, column).unwrap_or_default());
            }
        }
        text.truncate(text.trim_end().len());
        text
    }
}

impl Terminal {
    /// Copies the rows on the screen with the colors of `palette`, if the screen or the
    /// palette changed since the last copy into `frame`. Gives true for a new copy.
    pub fn frame(&self, frame: &mut Frame, palette: &Palette) -> bool {
        {
            let screen = self.shared.screen.lock();
            if frame.copied == Some((screen.generation, *palette)) {
                return false;
            }
            copy(&screen, palette, frame);
        }
        let mut io = self.io();
        if io.palette != *palette {
            // The answer to a color request names the colors that the user sees.
            io.palette = *palette;
        }
        true
    }
}

fn copy(screen: &Screen, palette: &Palette, frame: &mut Frame) {
    let term = &screen.term;
    let content = term.renderable_content();
    let (rows, columns) = (term.screen_lines(), term.columns());
    let offset = content.display_offset;
    frame.size = GridSize::new(u16::try_from(columns).unwrap_or(u16::MAX), u16::try_from(rows).unwrap_or(u16::MAX));
    frame.cells.clear();
    frame.marks.clear();
    for row in 0..rows {
        let line = Line(i32::try_from(row).unwrap_or(i32::MAX) - i32::try_from(offset).unwrap_or(i32::MAX));
        let cells = &term.grid()[line];
        for column in 0..columns {
            let cell = &cells[Column(column)];
            let selected = content.selection.is_some_and(|range| range.contains(Point::new(line, Column(column))));
            let mut fg = palette.resolve(cell.fg, content.colors);
            let mut bg = palette.resolve(cell.bg, content.colors);
            let flags = cell.flags;
            if flags.contains(Flags::DIM) {
                fg = dim(fg, bg);
            }
            if flags.contains(Flags::INVERSE) {
                std::mem::swap(&mut fg, &mut bg);
            }
            let spacer = flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER);
            let hidden = spacer || flags.contains(Flags::HIDDEN) || cell.c == '\t';
            let marks = cell.zerowidth().filter(|marks| !marks.is_empty() && !hidden);
            let mut own = CellFlags::default();
            own.put(CellFlags::BOLD, flags.contains(Flags::BOLD));
            own.put(CellFlags::ITALIC, flags.contains(Flags::ITALIC));
            own.put(CellFlags::UNDERLINE, flags.intersects(Flags::ALL_UNDERLINES));
            own.put(CellFlags::STRIKE, flags.contains(Flags::STRIKEOUT));
            own.put(CellFlags::WIDE, flags.contains(Flags::WIDE_CHAR));
            own.put(CellFlags::SPACER, spacer);
            own.put(CellFlags::SELECTED, selected);
            own.put(CellFlags::MARKS, marks.is_some());
            if let Some(marks) = marks {
                frame.marks.push((frame.cells.len(), marks.iter().collect()));
            }
            frame.cells.push(Cell { c: if hidden { ' ' } else { cell.c }, fg, bg, flags: own });
        }
    }
    frame.cursor =
        point_to_viewport(offset, content.cursor.point).filter(|point| point.line < rows).and_then(|point| {
            let shape = match content.cursor.shape {
                Shape::Block => CursorShape::Block,
                Shape::Underline => CursorShape::Underline,
                Shape::Beam => CursorShape::Beam,
                Shape::HollowBlock => CursorShape::HollowBlock,
                Shape::Hidden => return None,
            };
            let wide = term.grid()[content.cursor.point].flags.contains(Flags::WIDE_CHAR);
            Some(Cursor { row: point.line, column: point.column.0, shape, wide })
        });
    frame.modes = Modes::of(content.mode);
    frame.offset = offset;
    frame.history = term.history_size();
    frame.copied = Some((screen.generation, *palette));
}
