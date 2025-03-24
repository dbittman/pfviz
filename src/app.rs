use std::time::Duration;

use crate::{
    event::{AppEvent, Event, EventHandler},
    perf::PerfData,
    ui::Ui,
};
use ratatui::{
    DefaultTerminal,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};

/// Application.
#[derive(Debug)]
pub struct App {
    pub paused: bool,
    pub running: bool,
    pub events: EventHandler,
    pub ui: Ui,
    pub perf: PerfData,
}

impl App {
    /// Constructs a new instance of [`App`].
    pub fn new(mut perf: PerfData, width: u16, trace_file: impl ToString) -> Self {
        Self {
            running: true,
            paused: true,
            events: EventHandler::new(),
            ui: Ui::new(&mut perf, width, trace_file),
            perf,
        }
    }

    /// Run the application's main loop.
    pub fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        terminal.clear();
        while self.running {
            terminal.draw(|frame| frame.render_widget(&self, frame.area()))?;
            self.handle_events()?;
        }
        Ok(())
    }

    pub fn handle_events(&mut self) -> color_eyre::Result<()> {
        match self.events.next()? {
            Event::Tick => self.tick(),
            Event::Crossterm(event) => match event {
                crossterm::event::Event::Key(key_event) => self.handle_key_event(key_event)?,
                _ => {}
            },
            Event::App(app_event) => match app_event {
                AppEvent::Increment => self.increment_counter(),
                AppEvent::Decrement => self.decrement_counter(),
                AppEvent::TogglePause => self.set_pause(!self.paused),
                AppEvent::Quit => self.quit(),
            },
        }
        Ok(())
    }

    /// Handles the key events and updates the state of [`App`].
    pub fn handle_key_event(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {
        match key_event.code {
            KeyCode::Esc | KeyCode::Char('q') => self.events.send(AppEvent::Quit),
            KeyCode::Char('c' | 'C') if key_event.modifiers == KeyModifiers::CONTROL => {
                self.events.send(AppEvent::Quit)
            }
            KeyCode::Right => self.events.send(AppEvent::Increment),
            KeyCode::Left => self.events.send(AppEvent::Decrement),
            KeyCode::Char(' ') => self.events.send(AppEvent::TogglePause),
            // Other handlers you could add here.
            _ => {}
        }
        Ok(())
    }

    /// Handles the tick event of the terminal.
    ///
    /// The tick event is where you can update the state of your application with any logic that
    /// needs to be updated at a fixed frame rate. E.g. polling a server, updating an animation.
    pub fn tick(&mut self) {
        if self.paused {
            return;
        }
        let next_time = self.ui.status.cur_time + Duration::from_millis(10);
        while self.ui.status.cur_time < next_time {
            self.increment_counter();
            if self.ui.status.cur_event >= self.ui.status.num_events {
                break;
            }
        }
    }

    /// Set running to false to quit the application.
    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn increment_counter(&mut self) {
        if self.ui.status.cur_event >= self.ui.status.num_events {
            return;
        }
        let fault = &self.perf.faults[self.ui.status.cur_event];
        self.ui.fault_vis.fault(fault, &self.perf);
        self.ui
            .status
            .fault(self.ui.status.cur_event, fault, &self.perf);
        self.ui.status.cur_time = fault.time;
        if self.ui.status.cur_event < self.ui.status.num_events {
            self.ui.status.cur_event += 1;
        }
    }

    pub fn decrement_counter(&mut self) {}
    pub fn set_pause(&mut self, pause: bool) {
        self.paused = pause;
    }
}
