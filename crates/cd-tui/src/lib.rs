use std::io;
use std::path::PathBuf;
use std::time::Duration;

use cd_model::{Inventory, Progress};
use ratatui::crossterm::event::{self, Event, KeyEvent, KeyEventKind};
use ratatui::DefaultTerminal;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::task::JoinHandle;

use app::{App, Flow};

mod app;
mod detail;
mod export;
mod format;
mod logo;
mod logo_data;
mod table;
mod theme;
mod ui;

#[cfg(test)]
mod tests;

pub fn view(inventory: Inventory) -> io::Result<()> {
    let mut app = App::browse(inventory, false);
    app.truecolor = theme::truecolor();
    let mut terminal = ratatui::try_init()?;
    let result = run_view(&mut terminal, &mut app);
    ratatui::restore();
    result
}

fn run_view(terminal: &mut DefaultTerminal, app: &mut App) -> io::Result<()> {
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;
        if event::poll(Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press && app.key(key) == Flow::Quit {
                    return Ok(());
                }
            }
        }
        app.tick();
    }
}

pub struct Scanned {
    pub inventory: Inventory,
    pub saved: Option<PathBuf>,
}

pub async fn scan(
    mut progress: UnboundedReceiver<Progress>,
    mut scan: JoinHandle<Result<Inventory, String>>,
    save: bool,
) -> io::Result<Option<Scanned>> {
    let mut app = App::scanning();
    app.truecolor = theme::truecolor();
    app.auto_save = save;
    let mut terminal = ratatui::try_init()?;
    let (tx, mut input) = mpsc::unbounded_channel();
    std::thread::spawn(move || read_input(tx));
    let result = run_scan(
        &mut terminal,
        &mut app,
        &mut progress,
        &mut scan,
        &mut input,
    )
    .await;
    ratatui::restore();
    scan.abort();
    result.map(|()| {
        let saved = app.saved.take();
        app.into_inventory()
            .map(|inventory| Scanned { inventory, saved })
    })
}

enum Input {
    Key(KeyEvent),
    Tick,
}

fn read_input(tx: UnboundedSender<io::Result<Input>>) {
    loop {
        let input = match event::poll(Duration::from_millis(100)) {
            Ok(true) => match event::read() {
                Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => Ok(Input::Key(key)),
                Ok(_) => Ok(Input::Tick),
                Err(e) => Err(e),
            },
            Ok(false) => Ok(Input::Tick),
            Err(e) => Err(e),
        };
        let failed = input.is_err();
        if tx.send(input).is_err() || failed {
            return;
        }
    }
}

async fn run_scan(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    progress: &mut UnboundedReceiver<Progress>,
    scan: &mut JoinHandle<Result<Inventory, String>>,
    input: &mut UnboundedReceiver<io::Result<Input>>,
) -> io::Result<()> {
    let mut progress_open = true;
    let mut scan_open = true;
    loop {
        terminal.draw(|frame| ui::draw(frame, app))?;
        tokio::select! {
            event = progress.recv(), if progress_open => match event {
                Some(event) => app.progress(event),
                None => progress_open = false,
            },
            result = &mut *scan, if scan_open => {
                scan_open = false;
                let inventory = result.map_err(io::Error::other)?.map_err(io::Error::other)?;
                app.finished(inventory);
            }
            input = input.recv() => match input {
                Some(Ok(Input::Key(key))) => {
                    if app.key(key) == Flow::Quit {
                        return Ok(());
                    }
                }
                Some(Ok(Input::Tick)) => app.tick(),
                Some(Err(e)) => return Err(e),
                None => return Err(io::Error::other("terminal input closed")),
            },
        }
    }
}
