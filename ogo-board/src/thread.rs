//! Threads on the second core.

use std::thread::{self, JoinHandle};

use esp_idf_svc::hal::{cpu::Core, task::thread::ThreadSpawnConfiguration};

/// Starts a thread on the second core (APP CPU). The main task, and with it the game, runs on the
/// first; work that runs alongside, like sending frames to the display or feeding the speaker,
/// then doesn't take its time.
pub fn spawn_on_second_core<F, T>(
    name: &str,
    stack_size: usize,
    f: F,
) -> anyhow::Result<JoinHandle<T>>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    ThreadSpawnConfiguration {
        pin_to_core: Some(Core::Core1),
        ..Default::default()
    }
    .set()?;
    let handle = thread::Builder::new()
        .name(name.into())
        .stack_size(stack_size)
        .spawn(f);
    ThreadSpawnConfiguration::default().set()?;
    Ok(handle?)
}
