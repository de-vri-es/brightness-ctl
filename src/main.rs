//! Simple command line utility to control the brightness of your laptop screen on Linux.
//!
//! # Features
//!
//! * Can control the brightness of your laptop screen.
//! * Shows a notification with the new brightness.
//!   * Notification includes a progress bar if the notification daemon supports it!
//!
//! # Usage
//!
//! Usage information is available by running the program with the `--help` option:
//!
//! ```sh
//! brightness-ctl --help
//! ```

#[cfg(not(target_os = "linux"))]
compile_error!(concat!(
	"This tool currently only works on Linux.\n\n",
	"Support for additional platforms is highly appreciated.\n",
	"Feel free to open a PR on https://github.com/de-vri-es/brightness-ctl.\n\n",
));

use std::path::{Path, PathBuf};

use notify_rust::Notification;

const BACKLIGHT_CONTROLLER_DIR: &str = "/sys/class/backlight";

/// Set or get the brightness of your display.
#[derive(clap::Parser)]
#[clap(styles = clap_style())]
struct Options {
	/// Show more log messages.
	#[clap(long, short)]
	#[clap(global = true)]
	#[clap(action = clap::ArgAction::Count)]
	verbose: u8,

	/// Show less log messages.
	#[clap(long, short)]
	#[clap(global = true)]
	#[clap(action = clap::ArgAction::Count)]
	quiet: u8,

	/// The backlight controller to use.
	///
	/// Use the `list-controllers` command to get a list of available controllers.
	///
	/// If not specified, the first available controller is used.
	#[clap(long, short)]
	#[clap(global = true)]
	controller: Option<String>,

	/// The subcommand to execute.
	#[clap(subcommand)]
	command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
	/// Increase the screen brightness with the given percentage.
	Up {
		/// The percentage to increase the screen brightness with.
		#[clap(value_name = "VALUE")]
		value: f64,
	},

	/// Decrease the screen brightness with the given percentage.
	Down {
		/// The percentage to decrease the screen brightness with.
		#[clap(value_name = "VALUE")]
		value: f64,
	},

	/// Set the screen brightness to the given percentage.
	Set {
		/// The percentage to set the screen brightness to.
		#[clap(value_name = "VALUE")]
		value: f64,
	},

	/// Print the current screen brightness as a percentage.
	Get,

	/// Print a list of screen brightness controllers.
	ListControllers,
}

fn main() {
	if let Err(()) = do_main(clap::Parser::parse()) {
		std::process::exit(1);
	}
}

fn do_main(options: Options) -> Result<(), ()> {
	env_logger::Builder::new()
		.filter_module(module_path!(), log_level(options.verbose, options.quiet))
		.format_timestamp(None)
		.format_target(false)
		.parse_default_env()
		.init();

	if let Command::ListControllers = options.command {
		for controller in Controller::list()? {
			if let Some(name) = controller.file_name().map(|x| x.to_string_lossy()) {
				println!("{name}");
			}
		}
		return Ok(())
	}

	let mut controller = match &options.controller {
		Some(name) => Controller::open_by_name(name)?,
		None => Controller::open_first()?,
	};

	let mut brightness = controller.get_percentage();
	match options.command {
		Command::Up { value } => brightness += value,
		Command::Down { value } => brightness -= value,
		Command::Set { value } => brightness = value,
		Command::Get => {
			println!("{brightness:.0}");
			return Ok(())
		},
		Command::ListControllers => unreachable!(),
	}

	controller.set_percentage(brightness)?;
	show_notification(controller.get_percentage());
	Ok(())
}

#[derive(Debug)]
struct Controller{
	max: u64,
	value: u64,
	file: std::fs::File,
	path: PathBuf,
}

impl Controller {
	fn open(path: impl AsRef<Path>) -> Result<Self, ()> {
		let path = path.as_ref();
		log::debug!("Opening controller with path: {}", path.display());

		let path_max = path.join("max_brightness");
		let path_brightness = path.join("brightness");
		let mut file = std::fs::OpenOptions::new()
			.read(true)
			.write(true)
			.create(false)
			.truncate(false)
			.open(&path_brightness)
			.map_err(|e| log::error!("Failed to open {} for reading and writing: {e}", path_brightness.display()))?;
		let value = read_u64(&path_brightness, &mut file)?;
		let max = open_u64(&path_max)?;
		Ok(Self {
			max,
			value,
			file,
			path: path_brightness,
		})
	}

	fn open_by_name(name: &str) -> Result<Self, ()> {
		Self::open(Path::new(BACKLIGHT_CONTROLLER_DIR).join(name))
	}

	fn open_first() -> Result<Self, ()> {
		for path in Self::list()? {
			if let Ok(x) = Self::open(&path) {
				log::debug!("Using controller at {}", path.display());
				return Ok(x);
			}
		}

		log::error!("Failed to find any working congroller");
		Err(())
	}

	fn list() -> Result<impl Iterator<Item = PathBuf>, ()> {
		let path = BACKLIGHT_CONTROLLER_DIR;
		let dir = std::fs::read_dir(path)
			.map_err(|e| log::error!("Failed to open directory {path}: {e}"))?;
		Ok(dir.filter_map(move |entry| {
			let entry = entry
				.map_err(|e| log::error!("Failed to read entry of {path}: {e}"))
				.ok()?;
			Some(entry.path())
		}))
	}

	fn set_percentage(&mut self, value: f64) -> Result<(), ()> {
		use std::io::Write;

		let raw = (value / 100.0 * self.max as f64).round() as u64;
		let raw = raw.clamp(0, self.max);
		self.value = raw;
		self.file.write_all(raw.to_string().as_bytes())
			.map_err(|e| log::error!("Failed to write to {}: {e}", self.path.display()))?;
		Ok(())
	}

	fn get_percentage(&self) -> f64 {
		self.value as f64 / self.max as f64 * 100.0
	}
}

fn open_u64(path: &Path) -> Result<u64, ()> {
	let mut file = std::fs::File::open(path)
		.map_err(|e| log::error!("Failed to open {}: {e}", path.display()))?;
	read_u64(path, &mut file)
}

fn read_u64(path: &Path, file: &mut std::fs::File) -> Result<u64, ()> {
	use std::io::Read;
	let mut buffer = Vec::new();
	file.read_to_end(&mut buffer)
		.map_err(|e| log::error!("Failed to read from {}: {e}", path.display()))?;
	let data = std::str::from_utf8(&buffer)
		.map_err(|e| log::error!("Invalid UTF-8 in {}: {e}", path.display()))?;
	data.trim().parse()
		.map_err(|e| log::error!("Failed to parse {}: {e}", path.display()))
}

fn show_notification(percentage: f64) {
	let mut notification = Notification::new();
	notification.summary(&format!("Screen brightness: {percentage:.0}%"));
	notification.icon("display-brightness-symbolic");
	notification.id(0x49adff09);
	#[cfg(all(unix, not(target_os = "macos")))]
	notification.hint(notify_rust::Hint::CustomInt("value".to_owned(), percentage.round() as i32));
	notification.show()
		.map_err(|e| log::error!("Failed to show notification: {e}"))
		.ok();
}

/// Create a colorful style for the command line interface.
fn clap_style() -> clap::builder::Styles {
	use clap::builder::styling::AnsiColor;
	clap::builder::Styles::styled()
		.header(AnsiColor::Yellow.on_default())
		.usage(AnsiColor::Green.on_default())
		.literal(AnsiColor::Green.on_default())
		.placeholder(AnsiColor::Green.on_default())
}

/// Determine the log level filter based on the --verbose/-v and --quiet/-q flags.
fn log_level(verbose: u8, quiet: u8) -> log::LevelFilter {
	match i16::from(verbose) -  i16::from(quiet) {
		..=-2 => log::LevelFilter::Error,
		-1 => log::LevelFilter::Warn,
		0 => log::LevelFilter::Info,
		1 => log::LevelFilter::Debug,
		2.. => log::LevelFilter::Trace,
	}
}
