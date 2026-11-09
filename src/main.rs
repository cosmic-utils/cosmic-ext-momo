// SPDX-License-Identifier: {{LICENSE}}

use error::Error;

mod app;
mod cache;
mod client;
mod config;
mod error;
mod features;
mod i18n;
mod media;
mod persistence;
mod settings;
mod streaming;
mod ui;

fn main() -> Result<(), Error> {
    settings::init();
    cosmic::app::run::<app::AppModel>(settings::settings(), settings::flags()).map_err(Error::Iced)
}
