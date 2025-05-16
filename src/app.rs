use std::time::Duration;

use crate::{
    PlayCli,
    event::{AppEvent, Event, EventHandler, TICK_FPS},
    perf::FaultData,
    ui::Ui,
};
use ratatui::{
    DefaultTerminal,
    crossterm::event::{KeyCode, KeyEvent, KeyModifiers},
};

/// Application.
#[derive(Debug)]
pub struct App {
    pub running: bool,
    pub events: EventHandler,
    pub ui: Ui,
    pub data: FaultData,
    pub cli: PlayCli,
}

impl App {
    /// Constructs a new instance of [`App`].
    pub fn new(cli: PlayCli, data: FaultData) -> Self {
        Self {
            running: true,
            events: EventHandler::new(),
            ui: Ui::new(&cli, &data),
            data,
            cli,
        }
    }

    /// Run the application's main loop.
    pub fn run(mut self, mut terminal: DefaultTerminal) -> color_eyre::Result<()> {
        terminal.clear().unwrap();
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
                AppEvent::Increment => self.increment_counter(1),
                AppEvent::Decrement => self.decrement_counter(),
                AppEvent::TogglePause => self.set_pause(!self.ui.status.paused),
                AppEvent::Quit => self.quit(),
                AppEvent::MoveUp => self.ui.fault_vis.move_highlight(true),
                AppEvent::MoveDown => self.ui.fault_vis.move_highlight(false),
                AppEvent::Enter => self.ui.fault_vis.select(),
                AppEvent::Esc => {
                    if !self.ui.fault_vis.deselect() {
                        self.quit();
                    }
                }
                AppEvent::Char(c) => match c {
                    'b' => self.ui.fault_vis.toggle_break(),
                    'l' => self.ui.status.looping = !self.ui.status.looping,
                    '<' => self.goto_event(self.get_first_play_event()),
                    '>' => self.goto_event(self.get_last_play_event()),
                    ',' => {
                        if self
                            .ui
                            .status
                            .marker_a
                            .is_some_and(|a| a == self.ui.status.cur_event)
                        {
                            self.ui.status.marker_a = None;
                        } else {
                            self.ui.status.marker_a = Some(self.ui.status.cur_event);
                        }
                    }
                    '.' => {
                        if self
                            .ui
                            .status
                            .marker_b
                            .is_some_and(|b| b == self.ui.status.cur_event)
                        {
                            self.ui.status.marker_b = None;
                        } else {
                            self.ui.status.marker_b = Some(self.ui.status.cur_event);
                        }
                    }
                    _ => {}
                },
            },
        }
        Ok(())
    }

    /// Handles the key events and updates the state of [`App`].
    pub fn handle_key_event(&mut self, key_event: KeyEvent) -> color_eyre::Result<()> {
        match key_event.code {
            KeyCode::Char('q') => self.events.send(AppEvent::Quit),
            KeyCode::Char('c' | 'C') if key_event.modifiers == KeyModifiers::CONTROL => {
                self.events.send(AppEvent::Quit)
            }
            KeyCode::Right => self.events.send(AppEvent::Increment),
            KeyCode::Left => self.events.send(AppEvent::Decrement),

            KeyCode::Up => self.events.send(AppEvent::MoveUp),
            KeyCode::Down => self.events.send(AppEvent::MoveDown),
            KeyCode::Esc => self.events.send(AppEvent::Esc),
            KeyCode::Enter => self.events.send(AppEvent::Enter),
            KeyCode::Char('b') => self.events.send(AppEvent::Char('b')),
            KeyCode::Char('l') => self.events.send(AppEvent::Char('l')),
            KeyCode::Char(',') => self.events.send(AppEvent::Char(',')),
            KeyCode::Char('.') => self.events.send(AppEvent::Char('.')),
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
        if self.ui.status.paused {
            return;
        }
        if self.ui.status.looping {
            if self.ui.status.cur_event >= self.get_last_play_event() {
                self.ui.reset();
                self.goto_event(self.get_first_play_event());
            }
        }
        match self.cli.play_mode {
            crate::PlaybackMode::FrameTime => {
                let dt = Duration::from_secs_f64(self.cli.play_speed as f64);
                let next_time = self.ui.status.cur_time + dt;
                while self.ui.status.cur_time < next_time {
                    let count = self.count_events_before(next_time);
                    if count == 0 {
                        break;
                    }
                    self.increment_counter(count);
                    if self.ui.status.cur_event >= self.get_last_play_event() {
                        break;
                    }
                }
            }
            crate::PlaybackMode::FrameStep => {
                let mut i = 0;
                loop {
                    self.increment_counter(1);
                    i += 1;
                    if i as f32 > self.cli.play_speed {
                        break;
                    }
                    if self.ui.status.cur_event >= self.get_last_play_event() {
                        break;
                    }
                }
            }
            crate::PlaybackMode::Realtime => {
                let dt = 1.0 / TICK_FPS;
                let dt = Duration::from_secs_f64(dt * self.cli.play_speed as f64);
                let next_time = self.ui.status.cur_time + dt;
                while self.ui.status.cur_time < next_time {
                    let Some(dur) = self.next_event_time() else {
                        break;
                    };
                    if dur > next_time {
                        self.ui.status.cur_time = next_time;
                        break;
                    }
                    let count = self.count_events_before(next_time);
                    if count == 0 {
                        break;
                    }
                    self.increment_counter(count);
                    if self.ui.status.cur_event >= self.get_last_play_event() {
                        break;
                    }
                }
            }
        }
    }

    pub fn goto_event(&mut self, event: usize) {
        self.ui.status.cur_event = event;
        self.increment_counter(1);
    }

    pub fn get_last_play_event(&self) -> usize {
        if let Some(b) = self.ui.status.marker_b {
            b.min(self.ui.status.num_events)
        } else {
            self.ui.status.num_events
        }
    }

    pub fn get_first_play_event(&self) -> usize {
        self.ui.status.marker_a.unwrap_or(0)
    }

    /// Set running to false to quit the application.
    pub fn quit(&mut self) {
        self.running = false;
    }

    pub fn next_event_time(&self) -> Option<Duration> {
        if self.ui.status.cur_event >= self.get_last_play_event() {
            return None;
        }
        let fault = &self.data.records.slice()[self.ui.status.cur_event];
        Some(fault.time())
    }

    pub fn count_events_before(&self, time: Duration) -> usize {
        if self.ui.status.cur_event >= self.get_last_play_event() {
            return 0;
        }
        let faults =
            &self.data.records.slice()[self.ui.status.cur_event..self.get_last_play_event()];
        faults
            .iter()
            .position(|f| f.time() > time)
            .unwrap_or(faults.len())
    }

    pub fn increment_counter(&mut self, count: usize) {
        if self.ui.status.cur_event >= self.get_last_play_event() {
            return;
        }

        let faults =
            &self.data.records.slice()[self.ui.status.cur_event..self.get_last_play_event()];
        let count = count.min(faults.len());
        let faults = &faults[0..count];
        if faults.len() == 0 {
            return;
        }

        let res = self.ui.fault_vis.fault(faults, &self.data, &self.ui.map);
        if res.count == 0 {
            return;
        }
        self.ui.status.fault(
            self.ui.status.cur_event,
            faults,
            &self.data,
            res.hit_breakpoint,
        );
        self.ui.status.cur_time = faults[res.count - 1].time();
        if self.ui.status.cur_event < self.get_last_play_event() {
            self.ui.status.cur_event = self
                .get_last_play_event()
                .min(self.ui.status.cur_event + res.count);
        }

        if res.hit_breakpoint {
            self.set_pause(true);
        }
    }

    pub fn decrement_counter(&mut self) {}
    pub fn set_pause(&mut self, pause: bool) {
        self.ui.status.paused = pause;
    }
}
