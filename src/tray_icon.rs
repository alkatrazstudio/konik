// SPDX-License-Identifier: GPL-3.0-only
// 🄯 2023, Alexey Parfenov <zxed@alkatrazstudio.net>

use std::io::Cursor;
use std::sync::Arc;

use crate::project_info;
use anyhow::{Context, Result};
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;
use ksni::{Icon, MenuItem, ToolTip, Tray};
use png::Decoder;

#[derive(Copy, Clone)]
pub enum TrayIconImageType {
    Stop,
    Play,
    PlayHL,
    Pause,
}

pub struct TrayMenuItem {
    label: String,
    func: Arc<dyn Fn() + Send + Sync + 'static>,
}

impl TrayMenuItem {
    pub fn new<F>(label: &str, func: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        return Self {
            label: label.to_string(),
            func: Arc::new(func),
        };
    }
}

struct TrayIconData {
    stop_image: Icon,
    play_image: Icon,
    play_hl_image: Icon,
    pause_image: Icon,
    tooltip: String,
    image_type: TrayIconImageType,
    menu_items: Vec<TrayMenuItem>,
}

pub struct TrayIcon {
    handle: Handle<TrayIconData>,
    image_type: TrayIconImageType,
}

impl TrayIcon {
    fn rgba2argb(p: &mut [u8]) {
        p.copy_from_slice(&[p[3], p[0], p[1], p[2]]);
    }

    fn create_ico(bytes: &[u8]) -> Result<Icon> {
        let decoder = Decoder::new(Cursor::new(bytes));
        let mut reader = decoder.read_info().context("cannot read icon info")?;
        let mut buf = vec![
            0;
            reader
                .output_buffer_size()
                .context("reading PNG buffer size")?
        ];
        let info = reader
            .next_frame(&mut buf)
            .context("cannot read icon frame")?;
        let mut bytes = buf[..info.buffer_size()].to_vec();
        bytes.chunks_mut(4).for_each(Self::rgba2argb);
        return Ok(Icon {
            height: info.height as i32,
            width: info.width as i32,
            data: bytes,
        });
    }

    pub fn new() -> Result<Self> {
        let stop_image = Self::create_ico(include_bytes!("../img/stop.png"))
            .context("cannot create stop icon")?;
        let play_image = Self::create_ico(include_bytes!("../img/play.png"))
            .context("cannot create play icon")?;
        let play_hl_image = Self::create_ico(include_bytes!("../img/play_hl.png"))
            .context("cannot create play_hl icon")?;
        let pause_image = Self::create_ico(include_bytes!("../img/pause.png"))
            .context("cannot create pause icon")?;

        let data = TrayIconData {
            stop_image,
            play_image,
            play_hl_image,
            pause_image,
            tooltip: String::new(),
            image_type: TrayIconImageType::Stop,
            menu_items: vec![],
        };
        let handle = data.spawn()?;

        return Ok(Self {
            handle,
            image_type: TrayIconImageType::Stop,
        });
    }

    pub fn add_menu_item<F>(&self, menu_item_func: F)
    where
        F: Fn() -> TrayMenuItem,
    {
        self.handle.update(move |data| {
            data.menu_items.push(menu_item_func());
        });
    }

    pub fn play(&mut self) {
        if matches!(self.image_type, TrayIconImageType::Play) {
            return;
        }
        self.image_type = TrayIconImageType::Play;

        self.handle.update(|data| {
            data.image_type = TrayIconImageType::Play;
        });
    }

    pub fn play_hl(&mut self) {
        if matches!(self.image_type, TrayIconImageType::PlayHL) {
            return;
        }
        self.image_type = TrayIconImageType::PlayHL;

        self.handle.update(|data| {
            data.image_type = TrayIconImageType::PlayHL;
        });
    }

    pub fn stop(&mut self) {
        if matches!(self.image_type, TrayIconImageType::Stop) {
            return;
        }
        self.image_type = TrayIconImageType::Stop;

        self.handle.update(|data| {
            data.image_type = TrayIconImageType::Stop;
        });
    }

    pub fn pause(&mut self) {
        if matches!(self.image_type, TrayIconImageType::Pause) {
            return;
        }
        self.image_type = TrayIconImageType::Pause;

        self.handle.update(|data| {
            data.image_type = TrayIconImageType::Pause;
        });
    }

    pub fn image_type(&self) -> TrayIconImageType {
        return self.image_type;
    }

    pub fn set_tooltip(&self, text: &str) {
        self.handle.update(move |data| {
            data.tooltip = text.to_string();
        });
    }

    pub fn shutdown(&self) {
        self.handle.shutdown();
    }
}

impl Tray for TrayIconData {
    fn id(&self) -> String {
        return project_info::name().to_string();
    }

    fn title(&self) -> String {
        return project_info::title().to_string();
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        let image = match self.image_type {
            TrayIconImageType::Stop => self.stop_image.clone(),
            TrayIconImageType::Play => self.play_image.clone(),
            TrayIconImageType::PlayHL => self.play_hl_image.clone(),
            TrayIconImageType::Pause => self.pause_image.clone(),
        };
        return vec![image];
    }

    fn tool_tip(&self) -> ToolTip {
        return ToolTip {
            icon_name: self.icon_name(),
            icon_pixmap: self.icon_pixmap(),
            title: self.tooltip.clone(),
            description: String::new(),
        };
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        return self
            .menu_items
            .iter()
            .map(|m| {
                let f = m.func.clone();
                return MenuItem::Standard(StandardItem {
                    label: m.label.clone(),
                    activate: Box::new(move |_| f()),
                    ..Default::default()
                });
            })
            .collect();
    }
}
