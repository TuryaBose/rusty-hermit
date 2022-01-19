use rouille::Response;
use rust_embed::RustEmbed;

#[cfg(target_os = "hermit")]
extern crate hermit_sys;

#[derive(RustEmbed)]
#[folder = "slides"]
struct Slides;

fn main() {
	println!("Now listening on port 8000");

	rouille::start_server("0.0.0.0:8000", move |request| {
		let path = request.url();
		println!("Request {}", path);

		match path.as_str() {
			"/" => Slides::get("rwth.html")
				.map(|index| Response::from_data("text/html", index.data.as_ref())),
			_ => Slides::get(path.strip_prefix('/').unwrap()).map(|index| {
				Response::from_data(
					rouille::extension_to_mime(
						path.rsplit('/')
							.next()
							.and_then(|file| file.rsplit('.').next())
							.unwrap_or(""),
					),
					index.data.as_ref(),
				)
			}),
		}
		.unwrap_or_else(|| Response::empty_404())
	});
}
