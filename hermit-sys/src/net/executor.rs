use crate::net::{network_delay, network_poll};
use futures_lite::future::{self, FutureExt};
use futures_lite::pin;
use smoltcp::time::{Duration, Instant};
use std::future::Future;
use std::sync::{
	atomic::{AtomicBool, Ordering},
	Arc, Mutex,
};
use std::task::{Context, Poll, Wake};

/// A thread handle type
type Tid = u32;

extern "C" {
	fn sys_getpid() -> Tid;
	fn sys_yield();
	fn sys_wakeup_task(tid: Tid);
	fn sys_set_network_polling_mode(value: bool);
}

extern "Rust" {
	fn sys_block_current_task_with_timeout(timeout: Option<u64>);
}

thread_local! {
	static CURRENT_THREAD_NOTIFY: Arc<ThreadNotify> = {
		Arc::new(ThreadNotify::new())
	}
}

lazy_static! {
	static ref EXECUTOR: Mutex<SmoltcpExecutor> = Mutex::new(SmoltcpExecutor::new());
}

struct SmoltcpExecutor {
	pool: Vec<future::Boxed<()>>,
}

impl SmoltcpExecutor {
	pub fn new() -> Self {
		Self { pool: Vec::new() }
	}

	fn spawn_obj(&mut self, future: future::Boxed<()>) {
		self.pool.push(future);
	}
}

struct ThreadNotify {
	/// The (single) executor thread.
	thread: Tid,
	/// A flag to ensure a wakeup is not "forgotten" before the next `block_current_task`
	unparked: AtomicBool,
}

impl ThreadNotify {
	pub fn new() -> Self {
		Self {
			thread: unsafe { sys_getpid() },
			unparked: AtomicBool::new(false),
		}
	}
}

impl Drop for ThreadNotify {
	fn drop(&mut self) {
		println!("Dropping ThreadNotify!");
	}
}

impl Wake for ThreadNotify {
	fn wake(self: Arc<Self>) {
		self.wake_by_ref()
	}

	fn wake_by_ref(self: &Arc<Self>) {
		// Make sure the wakeup is remembered until the next `park()`.
		let unparked = self.unparked.swap(true, Ordering::Relaxed);
		if !unparked {
			unsafe {
				sys_wakeup_task(self.thread);
			}
		}
	}
}

// Set up and run a basic single-threaded spawner loop, invoking `f` on each
// turn.
fn run_until<T, F: FnMut(&mut Context<'_>) -> Poll<T>>(
	mut f: F,
	timeout: Option<Duration>,
) -> Result<T, ()> {
	unsafe {
		sys_set_network_polling_mode(true);
	}
	let start = Instant::now();

	CURRENT_THREAD_NOTIFY.with(|thread_notify| {
		let waker = thread_notify.clone().into();
		let mut cx = Context::from_waker(&waker);
		loop {
			if let Poll::Ready(t) = f(&mut cx) {
				unsafe {
					sys_set_network_polling_mode(false);
				}
				return Ok(t);
			}

			let now = Instant::now();

			if let Some(duration) = timeout {
				if now >= start + duration {
					unsafe {
						sys_set_network_polling_mode(false);
					}
					return Err(());
				}
			} else {
				let delay = network_delay(now).map(|d| d.total_millis());

				if delay.is_none() || delay.unwrap() > 100 {
					let unparked = thread_notify.unparked.swap(false, Ordering::Acquire);
					if !unparked {
						unsafe {
							sys_set_network_polling_mode(false);
							sys_block_current_task_with_timeout(delay);
							sys_yield();
							sys_set_network_polling_mode(true);
						}
						thread_notify.unparked.store(false, Ordering::Release);
						network_poll(&mut cx, Instant::now());
					}
				} else {
					network_poll(&mut cx, now);
				}
			}
		}
	})
}

pub fn block_on<F: Future>(f: F, timeout: Option<Duration>) -> Result<F::Output, ()> {
	pin!(f);
	run_until(|cx| f.as_mut().poll(cx), timeout)
}

pub fn spawn<F: Future<Output = ()> + std::marker::Send + 'static>(f: F) {
	EXECUTOR.lock().unwrap().spawn_obj(f.boxed())
}