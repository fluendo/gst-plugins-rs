// Copyright (C) 2025, Fluendo S.A.
//      Author: Ruben Gonzalez <rgonzalez@fluendo.com>
//
// This Source Code Form is subject to the terms of the Mozilla Public License, v2.0.
// If a copy of the MPL was not distributed with this file, You can obtain one at
// <https://mozilla.org/MPL/2.0/>.
//
// SPDX-License-Identifier: MPL-2.0

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Mode {
    #[default]
    Split,
    SideBySide,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Compositor {
    pub mode: Mode,
    pub zoom: usize,
    pub offset_x: i32,
    pub offset_y: i32,
    pub border: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy)]
pub struct Position {
    pub xpos: i32,
    pub ypos: i32,
    pub width: i32,
    pub height: i32,
    pub crop_right: i32,
    pub crop_left: i32,
}

const BORDER_STEP: usize = 10;
const WIDTH: i32 = 1280;
const HEIGHT: i32 = 720;
const HALF_WIDTH: i32 = WIDTH / 2;

impl Default for Compositor {
    fn default() -> Self {
        Self {
            mode: Mode::default(),
            zoom: 100,
            offset_x: 0,
            offset_y: 0,
            border: HALF_WIDTH,
            width: WIDTH,
            height: HEIGHT,
        }
    }
}

impl Compositor {
    #[allow(dead_code)]
    pub fn new(mode: Mode, width: i32, height: i32) -> Self {
        Self {
            mode,
            width,
            height,
            border: width / 2,
            ..Default::default()
        }
    }

    #[allow(dead_code)]
    pub fn new_side_by_side(width: i32, height: i32) -> Self {
        Self {
            mode: Mode::SideBySide,
            width,
            height,
            border: width / 2,
            ..Default::default()
        }
    }

    #[allow(dead_code)]
    pub fn new_split(width: i32, height: i32) -> Self {
        Self {
            mode: Mode::Split,
            width,
            height,
            border: width / 2,
            ..Default::default()
        }
    }

    /// Set side_by_side mode
    pub fn split_mode(&mut self) {
        self.mode = Mode::Split;
    }

    /// Set side_by_side mode
    pub fn side_by_side_mode(&mut self) {
        self.mode = Mode::SideBySide;
    }

    /// Set side_by_side mode
    #[allow(dead_code)]
    pub fn is_split_mode(&self) -> bool {
        self.mode == Mode::Split
    }

    /// Set side_by_side mode
    #[allow(dead_code)]
    pub fn is_side_by_side_mode(&self) -> bool {
        self.mode == Mode::SideBySide
    }

    /// Reset default values
    pub fn reset(&mut self) {
        let d = Compositor::default();
        self.zoom = d.zoom;
        self.offset_x = d.offset_x;
        self.offset_y = d.offset_y;
        self.border = d.border;
    }

    /// Reset only border to default values
    pub fn reset_border(&mut self) {
        let d = Compositor::default();
        self.border = d.border;
    }

    /// Reset only border to default values
    pub fn reset_position(&mut self) {
        let d = Compositor::default();
        self.zoom = d.zoom;
        self.offset_x = d.offset_x;
        self.offset_y = d.offset_y;
    }

    /// Moves the viewport by `x_step` pixels horizontally and `y_step` pixels vertically.
    /// Does not clamp the values, allowing offsets to exceed valid bounds.
    pub fn move_pos(&mut self, x_step: i32, y_step: i32) {
        self.offset_x += x_step;
        self.offset_y += y_step;
    }

    /// Set offset_x and offset_y
    pub fn move_pos_to(&mut self, x: i32, y: i32) {
        self.offset_x = x;
        self.offset_y = y;
    }

    /// Offsets the border inside the bounds.
    pub fn move_border(&mut self, offset: i32) {
        self.move_border_to(self.border + offset);
    }

    /// Set border position inside the bounds.
    pub fn move_border_to(&mut self, new_border: i32) {
        if new_border < 0 {
            self.border = 0
        } else if new_border > self.width {
            self.border = self.width
        } else {
            self.border = new_border
        }
    }

    /// Increases the zoom level, capping it at a sensible maximum (e.g., 1000000)
    pub fn zoom_in(&mut self) {
        let scale = if self.is_split_mode() { 2 } else { 4 };
        self.zoom_in_center_at(self.width / scale, self.height / 2);
    }

    /// Decreases the zoom level, ensuring it stays at a minimum of 1
    pub fn zoom_out(&mut self) {
        let scale = if self.is_split_mode() { 2 } else { 4 };
        self.zoom_out_center_at(self.width / scale, self.height / 2);
    }

    /// Increases the zoom level, capping it at a sensible maximum (e.g., 1000000)
    /// Update offset to keep centered
    pub fn zoom_in_center_at(&mut self, x: i32, y: i32) {
        self.zoom = (self.zoom + BORDER_STEP).min(1000000);
        self.fix_offset_when_zoom(x, y, true);
    }

    /// Decreases the zoom level, ensuring it stays at a minimum of 1,
    /// Update offset to keep centered
    pub fn zoom_out_center_at(&mut self, x: i32, y: i32) {
        self.zoom = (self.zoom.saturating_sub(BORDER_STEP)).max(1);
        self.fix_offset_when_zoom(x, y, false);
    }

    fn fix_offset_when_zoom(&mut self, x: i32, y: i32, inside: bool) {
        match self.mode {
            Mode::Split => {
                self.fix_offset_when_zoom_split(x, y, inside);
            }
            Mode::SideBySide => {
                self.fix_offset_when_zoom_side_by_side(x, y, inside);
            }
        }
    }

    fn fix_offset_when_zoom_split(&mut self, x: i32, y: i32, inside: bool) {
        let diff = x - (self.width / 2);
        let new_offset = diff / (BORDER_STEP as i32);
        if inside {
            self.offset_x -= new_offset;
        } else {
            self.offset_x += new_offset;
        }

        let diff = y - (self.height / 2);
        let new_offset = diff / (BORDER_STEP as i32);
        if inside {
            self.offset_y -= new_offset;
        } else {
            self.offset_y += new_offset;
        }
    }

    fn fix_offset_when_zoom_side_by_side(&mut self, x: i32, y: i32, inside: bool) {
        let x = x % (self.width / 2);
        let diff = x - (self.width / 4);
        let new_offset = diff / (BORDER_STEP as i32);
        if inside {
            self.offset_x -= new_offset;
        } else {
            self.offset_x += new_offset;
        }

        let diff = y - (self.height / 2);
        let new_offset = diff / (BORDER_STEP as i32);
        if inside {
            self.offset_y -= new_offset;
        } else {
            self.offset_y += new_offset;
        }
    }

    /// Calculates the two `Position`s for the input videos based on the compositor values
    pub fn get_positions(&self) -> (Position, Position) {
        match self.mode {
            Mode::Split => self.get_positions_split(),
            Mode::SideBySide => self.get_positions_side_by_side(),
        }
    }

    //here impl
    fn get_positions_side_by_side(&self) -> (Position, Position) {
        let zoom_factor = (self.zoom as f32) / 100.0;
        let viewport_width = (self.width as f32 * zoom_factor) as i32;
        let viewport_height = (self.height as f32 * zoom_factor) as i32;

        let half_width = self.width / 2;
        let half_viewport_width = viewport_width / 2;

        let pos_height = viewport_height / 2;
        let pos_ypos = self.offset_y + (self.height - pos_height) / 2;

        let pos_xpos = self.offset_x + (self.width - viewport_width) / 4;

        let unscaling = |w: i32| -> i32 {
            // crop is done over the original image
            let u_w = w * self.width / half_viewport_width;
            if u_w < self.width {
                u_w
            } else {
                0
            }
        };

        let pos0 = Position {
            xpos: if pos_xpos > half_width { 0 } else { pos_xpos },
            ypos: pos_ypos,
            width: if pos_xpos + half_viewport_width > half_width {
                if pos_xpos < half_width {
                    half_width - pos_xpos
                } else {
                    0
                }
            } else {
                half_viewport_width
            },
            height: pos_height,
            crop_right: if (pos_xpos + half_viewport_width) > half_width {
                let crop = pos_xpos + half_viewport_width - half_width;
                unscaling(crop)
            } else {
                0
            },
            crop_left: 0,
        };

        let pos1 = Position {
            xpos: if pos_xpos < 0 {
                half_width
            } else {
                pos_xpos + half_width
            },
            ypos: pos_ypos,
            width: if pos_xpos < 0 {
                if half_viewport_width > pos_xpos && half_viewport_width + pos_xpos > 0 {
                    half_viewport_width + pos_xpos
                } else {
                    0
                }
            } else {
                half_viewport_width
            },
            height: pos_height,
            crop_right: 0,
            crop_left: if pos_xpos > 0 || pos_xpos < -half_viewport_width {
                0
            } else {
                let crop = -pos_xpos;
                unscaling(crop)
            },
        };

        (pos0, pos1)
    }

    fn get_positions_split(&self) -> (Position, Position) {
        let zoom_factor = (self.zoom as f32) / 100.0;
        let viewport_width = (self.width as f32 * zoom_factor) as i32;
        let viewport_height = (self.height as f32 * zoom_factor) as i32;
        let viewport_offset_x = self.offset_x - (viewport_width - self.width) / 2;
        let viewport_offset_y = self.offset_y - (viewport_height - self.height) / 2;

        let pos0 = Position {
            xpos: if viewport_offset_x < self.border {
                viewport_offset_x
            } else {
                0
            },
            ypos: viewport_offset_y,
            width: {
                if viewport_offset_x < self.border {
                    if self.border - viewport_offset_x > viewport_width {
                        viewport_width
                    } else {
                        self.border - viewport_offset_x
                    }
                } else {
                    0
                }
            },
            height: viewport_height,
            crop_right: {
                if viewport_width + viewport_offset_x < self.border {
                    0
                } else if viewport_offset_x > self.border {
                    self.width
                } else {
                    // Note crop before zoom scaling (because glvideomixer implementation)w
                    let scale = self.width as f32 / viewport_width as f32;
                    let crop_right_scaled =
                        (viewport_width + viewport_offset_x - self.border) as f32;
                    (crop_right_scaled * scale) as i32
                }
            },
            crop_left: 0,
        };

        let pos1 = Position {
            xpos: if viewport_offset_x > self.border {
                viewport_offset_x
            } else {
                self.border
            },
            ypos: viewport_offset_y,
            width: {
                //TODO refactor
                if self.border < (viewport_width - viewport_offset_x) {
                    if viewport_width > self.border - viewport_offset_x {
                        if self.border < viewport_offset_x {
                            viewport_width
                        } else {
                            viewport_width - self.border + viewport_offset_x
                        }
                    } else {
                        0
                    }
                } else if self.border < viewport_offset_x {
                    viewport_width
                } else {
                    viewport_width - self.border + viewport_offset_x
                }
            },
            height: viewport_height,
            crop_right: 0,
            crop_left: {
                if viewport_width + viewport_offset_x < self.border {
                    self.width
                } else if viewport_offset_x > self.border {
                    0
                } else {
                    // Note crop before zoom scaling (because glvideomixer implementation)w
                    let scale = self.width as f32 / viewport_width as f32;
                    let crop_right_scaled = (self.border - viewport_offset_x) as f32;
                    (crop_right_scaled * scale) as i32
                }
            },
        };

        (pos0, pos1)
    }
}

#[cfg(test)]
mod tests {
    const HALF_HEIGHT: i32 = HEIGHT / 2;

    use super::*;

    #[test]
    fn test_compositor_default() {
        let compositor = Compositor::default();

        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");
    }

    #[test]
    fn test_compositor_new() {
        let width = 14;
        let height = 11;
        let compositor = Compositor::new(Mode::SideBySide, width, height);
        assert_eq!(compositor.mode, Mode::SideBySide, "compositor.mode");
        assert!(
            compositor.is_side_by_side_mode(),
            "compositor.is_side_by_side_mode"
        );
        assert_eq!(compositor.width, width, "compositor.width");
        assert_eq!(compositor.height, height, "compositor.height");

        let compositor = Compositor::new(Mode::Split, width, height);
        assert_eq!(compositor.mode, Mode::Split, "compositor.mode");
        assert!(compositor.is_split_mode(), "compositor.is_split_mode");
        assert_eq!(compositor.width, width, "compositor.width");
        assert_eq!(compositor.height, height, "compositor.height");

        let compositor = Compositor::new_split(width, height);
        assert!(compositor.is_split_mode(), "compositor.is_split_mode");

        let mut compositor = Compositor::new_side_by_side(width, height);
        assert!(
            compositor.is_side_by_side_mode(),
            "compositor.is_side_by_side_mode"
        );
        compositor.split_mode();
        assert!(compositor.is_split_mode(), "compositor.is_split_mode");
        compositor.side_by_side_mode();
        assert!(
            compositor.is_side_by_side_mode(),
            "compositor.is_side_by_side_mode"
        );
    }

    #[test]
    fn test_split_get_positions_default() {
        let compositor = Compositor::default();
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(pos0.xpos, 0, "pos0.xpos");
        assert_eq!(pos0.ypos, 0, "pos0.ypos");
        assert_eq!(pos0.width, HALF_WIDTH, "pos0.width");
        assert_eq!(pos0.height, HEIGHT, "pos0.height");
        assert_eq!(pos0.crop_right, HALF_WIDTH, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, HALF_WIDTH, "pos1.xpos");
        assert_eq!(pos1.ypos, 0, "pos1.ypos");
        assert_eq!(pos1.width, HALF_WIDTH, "pos1.width");
        assert_eq!(pos1.height, HEIGHT, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, HALF_WIDTH, "pos1.crop_left");
    }

    #[test]
    fn test_split_move_pos_left() {
        let mut compositor = Compositor::default();
        compositor.move_pos(-10, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, -10, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, -10, "pos0.xpos");
        assert_eq!(pos0.ypos, 0, "pos0.ypos");
        assert_eq!(pos0.width, HALF_WIDTH + 10, "pos0.width");
        assert_eq!(pos0.height, HEIGHT, "pos0.height");
        assert_eq!(pos0.crop_right, HALF_WIDTH - 10, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, HALF_WIDTH, "pos1.xpos");
        assert_eq!(pos1.ypos, 0, "pos1.ypos");
        assert_eq!(pos1.width, HALF_WIDTH - 10, "pos1.width");
        assert_eq!(pos1.height, HEIGHT, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, HALF_WIDTH + 10, "pos1.crop_left");

        compositor.reset_position();
        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");
    }

    #[test]
    fn test_split_move_pos_left_out_of_border() {
        let mut compositor = Compositor::default();
        compositor.move_pos(-1000, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, -1000, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, -1000, "pos0.xpos");
        assert_eq!(pos0.ypos, 0, "pos0.ypos");
        assert_eq!(pos0.width, WIDTH, "pos0.width");
        assert_eq!(pos0.height, HEIGHT, "pos0.height");
        assert_eq!(pos0.crop_right, 0, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, HALF_WIDTH, "pos1.xpos");
        assert_eq!(pos1.ypos, 0, "pos1.ypos");
        assert_eq!(pos1.width, 0, "pos1.width");
        assert_eq!(pos1.height, HEIGHT, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, WIDTH, "pos1.crop_left");
    }

    #[test]
    fn test_split_move_pos_right_out_of_border() {
        let mut compositor = Compositor::default();
        compositor.move_pos(1000, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 1000, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, 0, "pos0.xpos");
        assert_eq!(pos0.ypos, 0, "pos0.ypos");
        assert_eq!(pos0.width, 0, "pos0.width");
        assert_eq!(pos0.height, HEIGHT, "pos0.height");
        assert_eq!(pos0.crop_right, WIDTH, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, 1000, "pos1.xpos");
        assert_eq!(pos1.ypos, 0, "pos1.ypos");
        assert_eq!(pos1.width, WIDTH, "pos1.width");
        assert_eq!(pos1.height, HEIGHT, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 0, "pos1.crop_left");
    }

    #[test]
    fn test_split_move_border_left() {
        let mut compositor = Compositor::default();
        compositor.move_border(10);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH + 10, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, 0, "pos0.xpos");
        assert_eq!(pos0.ypos, 0, "pos0.ypos");
        assert_eq!(pos0.width, HALF_WIDTH + 10, "pos0.width");
        assert_eq!(pos0.height, HEIGHT, "pos0.height");
        assert_eq!(pos0.crop_right, HALF_WIDTH - 10, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, HALF_WIDTH + 10, "pos1.xpos");
        assert_eq!(pos1.ypos, 0, "pos1.ypos");
        assert_eq!(pos1.width, HALF_WIDTH - 10, "pos1.width");
        assert_eq!(pos1.height, HEIGHT, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, HALF_WIDTH + 10, "pos1.crop_left");

        compositor.reset_position();
        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH + 10, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");
    }

    #[test]
    fn test_split_zoom_in() {
        let mut compositor = Compositor::default();
        compositor.zoom_in();
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 110, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, -64, "pos0.xpos");
        assert_eq!(pos0.ypos, -36, "pos0.ypos");
        assert_eq!(pos0.width, 704, "pos0.width");
        assert_eq!(pos0.height, 792, "pos0.height");
        assert_eq!(pos0.crop_right, 640, "pos0.crop_right"); // Note crop before zoom scaling (because glvideomixer implementation) TODO delete
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, 640, "pos1.xpos");
        assert_eq!(pos1.ypos, -36, "pos1.ypos");
        assert_eq!(pos1.width, 704, "pos1.width");
        assert_eq!(pos1.height, 792, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 640, "pos1.crop_left"); // Note crop before zoom scaling (because glvideomixer implementation) TODO delete
    }

    #[test]
    fn test_split_zoom_out_five_times() {
        let mut compositor = Compositor::default();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 40, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, 384, "pos0.xpos");
        assert_eq!(pos0.ypos, 216, "pos0.ypos");
        assert_eq!(pos0.width, 256, "pos0.width");
        assert_eq!(pos0.height, 288, "pos0.height");
        assert_eq!(pos0.crop_right, 640, "pos0.crop_right"); // Note crop before zoom scaling (because glvideomixer implementation)
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, 640, "pos1.xpos");
        assert_eq!(pos1.ypos, 216, "pos1.ypos");
        assert_eq!(pos1.width, 256, "pos1.width");
        assert_eq!(pos1.height, 288, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 640, "pos1.crop_left");
    }

    #[test]
    fn test_split_zoom_out_five_times_and_move() {
        let mut compositor = Compositor::default();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 40, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, 384, "pos0.xpos");
        assert_eq!(pos0.ypos, 216, "pos0.ypos");
        assert_eq!(pos0.width, 256, "pos0.width");
        assert_eq!(pos0.height, 288, "pos0.height");
        assert_eq!(pos0.crop_right, 640, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, 640, "pos1.xpos");
        assert_eq!(pos1.ypos, 216, "pos1.ypos");
        assert_eq!(pos1.width, 256, "pos1.width");
        assert_eq!(pos1.height, 288, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 640, "pos1.crop_left");

        let current_width = pos0.width + pos1.width;
        compositor.move_pos(-10, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 40, "compositor.zoom");
        assert_eq!(compositor.offset_x, -10, "compositor.offset_x");
        assert_eq!(
            pos0.width + pos1.width,
            current_width,
            "pos0.width + pos1.width"
        );
        assert_eq!(pos0.crop_right, 615, "pos0.crop_right");
        assert_eq!(pos1.crop_left, 665, "pos1.crop_left");

        compositor.move_pos(-10, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 40, "compositor.zoom");
        assert_eq!(compositor.offset_x, -20, "compositor.offset_x");
        assert_eq!(
            pos0.width + pos1.width,
            current_width,
            "pos0.width + pos1.width"
        );
        assert_eq!(pos0.crop_right, 590, "pos0.crop_right");
        assert_eq!(pos1.crop_left, 690, "pos1.crop_left");

        compositor.move_pos(-10, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 40, "compositor.zoom");
        assert_eq!(compositor.offset_x, -30, "compositor.offset_x");
        assert_eq!(
            pos0.width + pos1.width,
            current_width,
            "pos0.width + pos1.width"
        );
        assert_eq!(pos0.crop_right, 565, "pos0.crop_right");
        assert_eq!(pos1.crop_left, 715, "pos1.crop_left");

        compositor.move_pos(-10, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 40, "compositor.zoom");
        assert_eq!(compositor.offset_x, -40, "compositor.offset_x");
        assert_eq!(
            pos0.width + pos1.width,
            current_width,
            "pos0.width + pos1.width"
        );
        assert_eq!(pos0.crop_right, 540, "pos0.crop_right");
        assert_eq!(pos1.crop_left, 740, "pos1.crop_left");
    }

    #[test]
    fn test_split_zoom_out_five_times_only_one_video() {
        let mut compositor = Compositor::default();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 40, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, 384, "pos0.xpos");
        assert_eq!(pos0.ypos, 216, "pos0.ypos");
        assert_eq!(pos0.width, 256, "pos0.width");
        assert_eq!(pos0.height, 288, "pos0.height");
        assert_eq!(pos0.crop_right, 640, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, 640, "pos1.xpos");
        assert_eq!(pos1.ypos, 216, "pos1.ypos");
        assert_eq!(pos1.width, 256, "pos1.width");
        assert_eq!(pos1.height, 288, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 640, "pos1.crop_left");

        compositor.move_border_to(0);
        let (pos0, pos1) = compositor.get_positions();
        assert_eq!(compositor.zoom, 40, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, 0, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, 0, "pos0.xpos");
        assert_eq!(pos0.ypos, 216, "pos0.ypos");
        assert_eq!(pos0.width, 0, "pos0.width");
        assert_eq!(pos0.height, 288, "pos0.height");
        assert_eq!(pos0.crop_right, 1280, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, 384, "pos1.xpos");
        assert_eq!(pos1.ypos, 216, "pos1.ypos");
        assert_eq!(pos1.width, 512, "pos1.width");
        assert_eq!(pos1.height, 288, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 0, "pos1.crop_left");
    }

    #[test]
    fn test_split_zoom_inout_center_at() {
        let mut compositor = Compositor::default();
        compositor.zoom_in_center_at(0, 0);

        assert_eq!(compositor.zoom, 110, "compositor.zoom");
        assert_eq!(compositor.offset_x, 64, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 36, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        compositor.zoom_out_center_at(0, 0);
        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        compositor.zoom_out_center_at(0, 0);
        assert_eq!(compositor.zoom, 90, "compositor.zoom");
        assert_eq!(compositor.offset_x, -64, "compositor.offset_x");
        assert_eq!(compositor.offset_y, -36, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");
    }

    #[test]
    fn test_split_new_zoom_inout_center_at() {
        let width = 12800;
        let height = 7200;
        let half_width = 12800 / 2;

        let mut compositor = Compositor::new(Mode::Split, width, height);
        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, half_width, "compositor.border");
        assert_eq!(compositor.width, width, "compositor.width");
        assert_eq!(compositor.height, height, "compositor.height");

        compositor.zoom_in_center_at(0, 0);

        assert_eq!(compositor.zoom, 110, "compositor.zoom");
        assert_eq!(compositor.offset_x, 640, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 360, "compositor.offset_y");
        assert_eq!(compositor.border, half_width, "compositor.border");
        assert_eq!(compositor.width, width, "compositor.width");
        assert_eq!(compositor.height, height, "compositor.height");

        compositor.zoom_out_center_at(0, 0);
        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, half_width, "compositor.border");
        assert_eq!(compositor.width, width, "compositor.width");
        assert_eq!(compositor.height, height, "compositor.height");

        compositor.zoom_out_center_at(0, 0);
        assert_eq!(compositor.zoom, 90, "compositor.zoom");
        assert_eq!(compositor.offset_x, -640, "compositor.offset_x");
        assert_eq!(compositor.offset_y, -360, "compositor.offset_y");
        assert_eq!(compositor.border, half_width, "compositor.border");
        assert_eq!(compositor.width, width, "compositor.width");
        assert_eq!(compositor.height, height, "compositor.height");
    }

    #[test]
    fn test_sidebyside_get_positions_default() {
        let mut compositor = Compositor::default();
        compositor.side_by_side_mode();

        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(pos0.xpos, 0, "pos0.xpos");
        assert_eq!(pos0.ypos, HEIGHT / 4, "pos0.ypos");
        assert_eq!(pos0.width, HALF_WIDTH, "pos0.width");
        assert_eq!(pos0.height, HEIGHT / 2, "pos0.height");
        assert_eq!(pos0.crop_right, 0, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, HALF_WIDTH, "pos1.xpos");
        assert_eq!(pos1.ypos, HEIGHT / 4, "pos1.ypos");
        assert_eq!(pos1.width, HALF_WIDTH, "pos1.width");
        assert_eq!(pos1.height, HEIGHT / 2, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 0, "pos1.crop_left");
    }

    #[test]
    fn test_sidebyside_move_pos_up_reset_down() {
        let mut compositor = Compositor::default();
        compositor.side_by_side_mode();

        compositor.move_pos(0, -10);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, -10, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, 0, "pos0.xpos");
        assert_eq!(pos0.ypos, 170, "pos0.ypos");
        assert_eq!(pos0.width, 640, "pos0.width");
        assert_eq!(pos0.height, HALF_HEIGHT, "pos0.height");
        assert_eq!(pos0.crop_right, 0, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, HALF_WIDTH, "pos1.xpos");
        assert_eq!(pos1.ypos, 170, "pos1.ypos");
        assert_eq!(pos1.width, 640, "pos1.width");
        assert_eq!(pos1.height, HALF_HEIGHT, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 0, "pos1.crop_left");

        compositor.reset_position();
        assert!(
            compositor.is_side_by_side_mode(),
            "compositor.is_side_by_side_mode"
        );
        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        compositor.move_pos(0, 10);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 10, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, 0, "pos0.xpos");
        assert_eq!(pos0.ypos, 190, "pos0.ypos");
        assert_eq!(pos0.width, 640, "pos0.width");
        assert_eq!(pos0.height, HALF_HEIGHT, "pos0.height");
        assert_eq!(pos0.crop_right, 0, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, HALF_WIDTH, "pos1.xpos");
        assert_eq!(pos1.ypos, 190, "pos1.ypos");
        assert_eq!(pos1.width, 640, "pos1.width");
        assert_eq!(pos1.height, HALF_HEIGHT, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 0, "pos1.crop_left");
    }

    #[test]
    fn test_sidebyside_move_pos_left_reset_right() {
        let mut compositor = Compositor::default();
        compositor.side_by_side_mode();

        compositor.move_pos(-10, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, -10, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, -10, "pos0.xpos");
        assert_eq!(pos0.ypos, 180, "pos0.ypos");
        assert_eq!(pos0.width, 640, "pos0.width");
        assert_eq!(pos0.height, HALF_HEIGHT, "pos0.height");
        assert_eq!(pos0.crop_right, 0, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, HALF_WIDTH, "pos1.xpos");
        assert_eq!(pos1.ypos, 180, "pos1.ypos");
        assert_eq!(pos1.width, 630, "pos1.width");
        assert_eq!(pos1.height, HALF_HEIGHT, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 20, "pos1.crop_left");

        compositor.reset_position();
        assert!(
            compositor.is_side_by_side_mode(),
            "compositor.is_side_by_side_mode"
        );
        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        compositor.move_pos(10, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 10, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, 10, "pos0.xpos");
        assert_eq!(pos0.ypos, 180, "pos0.ypos");
        assert_eq!(pos0.width, 630, "pos0.width");
        assert_eq!(pos0.height, HALF_HEIGHT, "pos0.height");
        assert_eq!(pos0.crop_right, 20, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, HALF_WIDTH + 10, "pos1.xpos");
        assert_eq!(pos1.ypos, 180, "pos1.ypos");
        assert_eq!(pos1.width, 640, "pos1.width");
        assert_eq!(pos1.height, HALF_HEIGHT, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 0, "pos1.crop_left");
    }

    #[test]
    fn test_sidebyside_move_pos_left_out_of_border() {
        let mut compositor = Compositor::default();
        compositor.side_by_side_mode();

        compositor.move_pos(-1000, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, -1000, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, -1000, "pos0.xpos");
        assert_eq!(pos0.ypos, 180, "pos0.ypos");
        assert_eq!(pos0.width, HALF_WIDTH, "pos0.width");
        assert_eq!(pos0.height, HALF_HEIGHT, "pos0.height");
        assert_eq!(pos0.crop_right, 0, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, HALF_WIDTH, "pos1.xpos");
        assert_eq!(pos1.ypos, 180, "pos1.ypos");
        assert_eq!(pos1.width, 0, "pos1.width");
        assert_eq!(pos1.height, HALF_HEIGHT, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 0, "pos1.crop_left");
    }

    #[test]
    fn test_sidebyside_move_pos_right_out_of_border() {
        let mut compositor = Compositor::default();
        compositor.side_by_side_mode();

        compositor.move_pos(1000, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 1000, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, 0, "pos0.xpos");
        assert_eq!(pos0.ypos, 180, "pos0.ypos");
        assert_eq!(pos0.width, 0, "pos0.width");
        assert_eq!(pos0.height, HALF_HEIGHT, "pos0.height");
        assert_eq!(pos0.crop_right, 0, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, 1640, "pos1.xpos");
        assert_eq!(pos1.ypos, 180, "pos1.ypos");
        assert_eq!(pos1.width, HALF_WIDTH, "pos1.width");
        assert_eq!(pos1.height, HALF_HEIGHT, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 0, "pos1.crop_left");
    }

    #[test]
    fn test_sidebyside_zoom_in() {
        let mut compositor = Compositor::default();
        compositor.side_by_side_mode();

        compositor.zoom_in();
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 110, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, -32, "pos0.xpos");
        assert_eq!(pos0.ypos, 162, "pos0.ypos");
        assert_eq!(pos0.width, 704 - 32, "pos0.width");
        assert_eq!(pos0.height, 396, "pos0.height");
        assert_eq!(pos0.crop_right, 58, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, 640, "pos1.xpos");
        assert_eq!(pos1.ypos, 162, "pos1.ypos");
        assert_eq!(pos1.width, 672, "pos1.width");
        assert_eq!(pos1.height, 396, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 58, "pos1.crop_left");
    }

    #[test]
    fn test_sidebyside_zoom_out_five_times() {
        let mut compositor = Compositor::default();
        compositor.side_by_side_mode();

        compositor.zoom_out();
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 90, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, 32, "pos0.xpos");
        assert_eq!(pos0.ypos, 198, "pos0.ypos");
        assert_eq!(pos0.width, 576, "pos0.width");
        assert_eq!(pos0.height, 324, "pos0.height");
        assert_eq!(pos0.crop_right, 0, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, 640 + 32, "pos1.xpos");
        assert_eq!(pos1.ypos, 198, "pos1.ypos");
        assert_eq!(pos1.width, 576, "pos1.width");
        assert_eq!(pos1.height, 324, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 0, "pos1.crop_left");

        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 40, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, 192, "pos0.xpos");
        assert_eq!(pos0.ypos, 288, "pos0.ypos");
        assert_eq!(pos0.width, 256, "pos0.width");
        assert_eq!(pos0.height, 144, "pos0.height");
        assert_eq!(pos0.crop_right, 0, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, 640 + 192, "pos1.xpos");
        assert_eq!(pos1.ypos, 288, "pos1.ypos");
        assert_eq!(pos1.width, 256, "pos1.width");
        assert_eq!(pos1.height, 144, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 0, "pos1.crop_left");
    }

    #[test]
    fn test_sidebyside_zoom_in_and_move() {
        let mut compositor = Compositor::default();
        compositor.side_by_side_mode();

        compositor.zoom_in();
        compositor.move_pos(-20, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 110, "compositor.zoom");
        assert_eq!(compositor.offset_x, -20, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, -52, "pos0.xpos");
        assert_eq!(pos0.ypos, 162, "pos0.ypos");
        assert_eq!(pos0.width, 692, "pos0.width");
        assert_eq!(pos0.height, 396, "pos0.height");
        assert_eq!(pos0.crop_right, 21, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, 640, "pos1.xpos");
        assert_eq!(pos1.ypos, 162, "pos1.ypos");
        assert_eq!(pos1.width, 704 - 52, "pos1.width");
        assert_eq!(pos1.height, 396, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 94, "pos1.crop_left");
    }

    #[test]
    fn test_sidebyside_zoom_out_five_times_and_move() {
        let mut compositor = Compositor::default();
        compositor.side_by_side_mode();

        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        compositor.zoom_out();
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 40, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, 192, "pos0.xpos");
        assert_eq!(pos0.ypos, 288, "pos0.ypos");
        assert_eq!(pos0.width, 256, "pos0.width");
        assert_eq!(pos0.height, 144, "pos0.height");
        assert_eq!(pos0.crop_right, 0, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, 640 + 192, "pos1.xpos");
        assert_eq!(pos1.ypos, 288, "pos1.ypos");
        assert_eq!(pos1.width, 256, "pos1.width");
        assert_eq!(pos1.height, 144, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 0, "pos1.crop_left");

        let current_width = pos0.width + pos1.width;
        compositor.move_pos(-10, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 40, "compositor.zoom");
        assert_eq!(compositor.offset_x, -10, "compositor.offset_x");
        assert_eq!(
            pos0.width + pos1.width,
            current_width,
            "pos0.width + pos1.width"
        );
        assert_eq!(pos1.xpos, 640 + 192 - 10, "pos1.xpos");
        assert_eq!(pos0.crop_right, 0, "pos0.crop_right");
        assert_eq!(pos1.crop_left, 0, "pos1.crop_left");

        compositor.move_pos(-10, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 40, "compositor.zoom");
        assert_eq!(compositor.offset_x, -20, "compositor.offset_x");
        assert_eq!(
            pos0.width + pos1.width,
            current_width,
            "pos0.width + pos1.width"
        );
        assert_eq!(pos0.crop_right, 0, "pos0.crop_right");
        assert_eq!(pos1.crop_left, 0, "pos1.crop_left");

        compositor.move_pos(-10, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 40, "compositor.zoom");
        assert_eq!(compositor.offset_x, -30, "compositor.offset_x");
        assert_eq!(
            pos0.width + pos1.width,
            current_width,
            "pos0.width + pos1.width"
        );
        assert_eq!(pos0.crop_right, 0, "pos0.crop_right");
        assert_eq!(pos1.crop_left, 0, "pos1.crop_left");

        compositor.move_pos(-200, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 40, "compositor.zoom");
        assert_eq!(compositor.offset_x, -230, "compositor.offset_x");

        assert_eq!(pos0.xpos, -38, "pos0.xpos");
        assert_eq!(pos0.ypos, 288, "pos0.ypos");
        assert_eq!(pos0.width, 256, "pos0.width");
        assert_eq!(pos0.height, 144, "pos0.height");
        assert_eq!(pos0.crop_right, 0, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, 640, "pos1.xpos");
        assert_eq!(pos1.ypos, 288, "pos1.ypos");
        assert_eq!(pos1.width, 218, "pos1.width");
        assert_eq!(pos1.height, 144, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 190, "pos1.crop_left");
    }

    #[test]
    fn test_sidebyside_zoom_inout_center_at() {
        let mut compositor = Compositor::default();
        compositor.side_by_side_mode();

        compositor.zoom_in_center_at(0, 0);

        assert_eq!(compositor.zoom, 110, "compositor.zoom");
        assert_eq!(compositor.offset_x, 32, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 36, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        compositor.zoom_out_center_at(0, 0);
        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        compositor.zoom_out_center_at(0, 0);
        assert_eq!(compositor.zoom, 90, "compositor.zoom");
        assert_eq!(compositor.offset_x, -32, "compositor.offset_x");
        assert_eq!(compositor.offset_y, -36, "compositor.offset_y");
        assert_eq!(compositor.border, HALF_WIDTH, "compositor.border");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");
    }

    #[test]
    fn test_sidebyside_new_zoom_inout_center_at() {
        let width = 12800;
        let height = 7200;
        let half_width = 12800 / 2;

        let mut compositor = Compositor::new(Mode::Split, width, height);
        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, half_width, "compositor.border");
        assert_eq!(compositor.width, width, "compositor.width");
        assert_eq!(compositor.height, height, "compositor.height");

        compositor.zoom_in_center_at(0, 0);

        assert_eq!(compositor.zoom, 110, "compositor.zoom");
        assert_eq!(compositor.offset_x, 640, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 360, "compositor.offset_y");
        assert_eq!(compositor.border, half_width, "compositor.border");
        assert_eq!(compositor.width, width, "compositor.width");
        assert_eq!(compositor.height, height, "compositor.height");

        compositor.zoom_out_center_at(0, 0);
        assert_eq!(compositor.zoom, 100, "compositor.zoom");
        assert_eq!(compositor.offset_x, 0, "compositor.offset_x");
        assert_eq!(compositor.offset_y, 0, "compositor.offset_y");
        assert_eq!(compositor.border, half_width, "compositor.border");
        assert_eq!(compositor.width, width, "compositor.width");
        assert_eq!(compositor.height, height, "compositor.height");

        compositor.zoom_out_center_at(0, 0);
        assert_eq!(compositor.zoom, 90, "compositor.zoom");
        assert_eq!(compositor.offset_x, -640, "compositor.offset_x");
        assert_eq!(compositor.offset_y, -360, "compositor.offset_y");
        assert_eq!(compositor.border, half_width, "compositor.border");
        assert_eq!(compositor.width, width, "compositor.width");
        assert_eq!(compositor.height, height, "compositor.height");
    }

    #[test]
    fn test_sidebyside_bug_1() {
        let mut compositor = Compositor {
            mode: Mode::SideBySide,
            zoom: 320,
            offset_x: 315,
            offset_y: -103,
            ..Default::default()
        };

        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 320, "compositor.zoom");
        assert_eq!(compositor.offset_x, 315, "compositor.offset_x");
        assert_eq!(compositor.offset_y, -103, "compositor.offset_y");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, -389, "pos0.xpos");
        assert_eq!(pos0.ypos, -319, "pos0.ypos");
        assert_eq!(pos0.width, 1029, "pos0.width");
        assert_eq!(pos0.height, 1152, "pos0.height");
        assert_eq!(pos0.crop_right, 636, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, 640, "pos1.xpos");
        assert_eq!(pos1.ypos, -319, "pos1.ypos");
        assert_eq!(pos1.width, 1659, "pos1.width");
        assert_eq!(pos1.height, 1152, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 243, "pos1.crop_left");

        compositor.move_pos(30, 0);
        let (pos0, pos1) = compositor.get_positions();

        assert_eq!(compositor.zoom, 320, "compositor.zoom");
        assert_eq!(compositor.offset_x, 345, "compositor.offset_x");
        assert_eq!(compositor.offset_y, -103, "compositor.offset_y");
        assert_eq!(compositor.width, WIDTH, "compositor.width");
        assert_eq!(compositor.height, HEIGHT, "compositor.height");

        assert_eq!(pos0.xpos, -359, "pos0.xpos");
        assert_eq!(pos0.ypos, -319, "pos0.ypos");
        assert_eq!(pos0.width, 999, "pos0.width");
        assert_eq!(pos0.height, 1152, "pos0.height");
        assert_eq!(pos0.crop_right, 655, "pos0.crop_right");
        assert_eq!(pos0.crop_left, 0, "pos0.crop_left");

        assert_eq!(pos1.xpos, 640, "pos1.xpos");
        assert_eq!(pos1.ypos, -319, "pos1.ypos");
        assert_eq!(pos1.width, 1689, "pos1.width");
        assert_eq!(pos1.height, 1152, "pos1.height");
        assert_eq!(pos1.crop_right, 0, "pos1.crop_right");
        assert_eq!(pos1.crop_left, 224, "pos1.crop_left");
    }
}
