//! Glass that shows what lies beneath it. The browser frame paints above the
//! page, so before the frame's first pane the screen painted so far is copied
//! into a mipmapped texture. Each pane then draws that copy blurred within its
//! rounded shape and bent near its rim like the edge of a lens; the pane's tint
//! and highlights paint on top. OpenGL calls are this module's only unsafe
//! code. Without OpenGL 3 or OpenGL ES 3 shaders, panes keep their plain tint.
use eframe::{
    egui::{self, CornerRadius, PaintCallback, PaintCallbackInfo, Rect, Shape},
    egui_glow,
    glow::{self, HasContext as _},
};
use std::sync::{Arc, Mutex};

/// How a pane blurs and bends the page behind it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lens {
    /// Blur radius, in points.
    pub blur: f32,
    /// How far the rim bends the backdrop outward, in points.
    pub refraction: f32,
    /// Screen heights, in points, where the glass is whole and where it has
    /// faded away, for panes that soften into the page.
    pub fade: Option<(f32, f32)>,
}

/// The captured screen, shared by one context's paint callbacks.
#[derive(Default)]
struct State {
    resources: Option<Resources>,
    /// The renderer cannot draw lenses; panes keep their plain tint.
    unsupported: bool,
    /// The pass whose screen the texture holds.
    captured: Option<u64>,
}

struct Resources {
    program: glow::Program,
    vertex_array: glow::VertexArray,
    texture: glow::Texture,
    size: [i32; 2],
    backdrop: Option<glow::UniformLocation>,
    texture_size: Option<glow::UniformLocation>,
    rect: Option<glow::UniformLocation>,
    radii: Option<glow::UniformLocation>,
    blur: Option<glow::UniformLocation>,
    refraction: Option<glow::UniformLocation>,
    fade: Option<glow::UniformLocation>,
}

type Shared = Arc<Mutex<State>>;

fn shared(ctx: &egui::Context) -> Shared {
    ctx.data_mut(|data| {
        data.get_temp_mut_or_default::<Shared>(egui::Id::unique("olive-glass-lens"))
            .clone()
    })
}

/// Copy the screen painted so far, for the panes painted after it this pass.
pub fn capture(ctx: &egui::Context, screen: Rect) -> Shape {
    let state = shared(ctx);
    let pass = ctx.cumulative_pass_nr();
    Shape::Callback(PaintCallback {
        rect: screen,
        callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
            if let Ok(mut state) = state.lock() {
                capture_screen(&mut state, painter.gl(), info.screen_size_px, pass);
            }
        })),
    })
}

/// The captured screen behind `rect`, seen through `lens`. Panes painted
/// before this pass's capture draw nothing.
pub fn pane(ctx: &egui::Context, rect: Rect, radius: CornerRadius, lens: Lens) -> Shape {
    let state = shared(ctx);
    let pass = ctx.cumulative_pass_nr();
    Shape::Callback(PaintCallback {
        // A little extra room, as the callback viewport is rounded to pixels.
        rect: rect.expand(1.0),
        callback: Arc::new(egui_glow::CallbackFn::new(move |info, painter| {
            if let Ok(state) = state.lock() {
                draw_pane(&state, painter.gl(), &info, rect, radius, lens, pass);
            }
        })),
    })
}

fn capture_screen(state: &mut State, gl: &glow::Context, screen_px: [u32; 2], pass: u64) {
    if state.unsupported {
        return;
    }
    if state.resources.is_none() {
        // SAFETY: the callback runs on the thread that owns the current context.
        match unsafe { create(gl) } {
            Ok(resources) => state.resources = Some(resources),
            Err(_) => {
                state.unsupported = true;
                return;
            }
        }
    }
    let Some(resources) = state.resources.as_mut() else {
        return;
    };
    let [width, height] = screen_px.map(|side| side.min(i32::MAX as u32) as i32);
    if width == 0 || height == 0 {
        return;
    }
    // SAFETY: plain texture calls on objects this module created; egui binds
    // its own texture again before its next mesh.
    unsafe {
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(resources.texture));
        if resources.size != [width, height] {
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                width,
                height,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            resources.size = [width, height];
        }
        gl.copy_tex_sub_image_2d(glow::TEXTURE_2D, 0, 0, 0, 0, 0, width, height);
        gl.generate_mipmap(glow::TEXTURE_2D);
        gl.bind_texture(glow::TEXTURE_2D, None);
    }
    state.captured = Some(pass);
}

fn draw_pane(
    state: &State,
    gl: &glow::Context,
    info: &PaintCallbackInfo,
    rect: Rect,
    radius: CornerRadius,
    lens: Lens,
    pass: u64,
) {
    if state.captured != Some(pass) {
        return;
    }
    let Some(resources) = &state.resources else {
        return;
    };
    let scale = info.pixels_per_point;
    let height = info.screen_size_px[1] as f32;
    // OpenGL counts rows from the bottom of the screen.
    let y = |points: f32| height - points * scale;
    let fade = lens
        .fade
        .map_or([0.0, 0.0], |(whole, clear)| [y(whole), y(clear)]);
    // Upper right, lower right, upper left, lower left.
    let radii = [radius.ne, radius.se, radius.nw, radius.sw].map(|r| f32::from(r) * scale);
    // SAFETY: draws with objects this module created; egui restores its own
    // program, vertex array and blending after the callback.
    unsafe {
        gl.use_program(Some(resources.program));
        gl.bind_vertex_array(Some(resources.vertex_array));
        gl.active_texture(glow::TEXTURE0);
        gl.bind_texture(glow::TEXTURE_2D, Some(resources.texture));
        gl.uniform_1_i32(resources.backdrop.as_ref(), 0);
        gl.uniform_2_f32(
            resources.texture_size.as_ref(),
            resources.size[0] as f32,
            resources.size[1] as f32,
        );
        gl.uniform_4_f32(
            resources.rect.as_ref(),
            rect.min.x * scale,
            y(rect.max.y),
            rect.max.x * scale,
            y(rect.min.y),
        );
        gl.uniform_4_f32(
            resources.radii.as_ref(),
            radii[0],
            radii[1],
            radii[2],
            radii[3],
        );
        gl.uniform_1_f32(resources.blur.as_ref(), lens.blur * scale);
        gl.uniform_1_f32(resources.refraction.as_ref(), lens.refraction * scale);
        gl.uniform_2_f32(resources.fade.as_ref(), fade[0], fade[1]);
        gl.draw_arrays(glow::TRIANGLES, 0, 3);
        gl.bind_vertex_array(None);
        gl.bind_texture(glow::TEXTURE_2D, None);
    }
}

/// A triangle covering the callback's viewport.
const VERTEX: &str = r"
void main() {
    vec2 corner = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    gl_Position = vec4(corner * 2.0 - 1.0, 0.0, 1.0);
}
";

/// The pane's rounded shape, from a signed distance in pixels; near the rim
/// the backdrop bends outward, then a Gaussian-weighted spiral of samples from
/// a coarse mip level blurs it.
const FRAGMENT: &str = r"
uniform sampler2D u_backdrop;
uniform vec2 u_texture_size;
uniform vec4 u_rect;
uniform vec4 u_radii;
uniform float u_blur;
uniform float u_refraction;
uniform vec2 u_fade;
out vec4 frag_color;

float rounded_box(vec2 p, vec2 half_size, vec4 radii) {
    radii.xy = p.x > 0.0 ? radii.xy : radii.zw;
    float r = p.y > 0.0 ? radii.x : radii.y;
    vec2 q = abs(p) - half_size + r;
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0)) - r;
}

void main() {
    vec2 p = gl_FragCoord.xy;
    vec2 center = 0.5 * (u_rect.xy + u_rect.zw);
    vec2 half_size = 0.5 * (u_rect.zw - u_rect.xy);
    float d = rounded_box(p - center, half_size, u_radii);
    float coverage = clamp(0.5 - d, 0.0, 1.0);
    if (u_fade.x != u_fade.y) {
        coverage *= clamp((p.y - u_fade.y) / (u_fade.x - u_fade.y), 0.0, 1.0);
    }
    if (coverage <= 0.0) {
        discard;
    }
    vec2 dx = vec2(1.0, 0.0);
    vec2 dy = vec2(0.0, 1.0);
    vec2 normal = vec2(
        rounded_box(p + dx - center, half_size, u_radii) - rounded_box(p - dx - center, half_size, u_radii),
        rounded_box(p + dy - center, half_size, u_radii) - rounded_box(p - dy - center, half_size, u_radii));
    normal /= max(length(normal), 0.0001);
    float bevel = max(min(min(half_size.x, half_size.y), 3.0 * u_refraction), 1.0);
    float rim = clamp(1.0 + d / bevel, 0.0, 1.0);
    vec2 q = p + normal * u_refraction * rim * rim;
    float lod = log2(max(u_blur * 0.25, 1.0));
    vec3 sum = vec3(0.0);
    float total = 0.0;
    for (int i = 0; i < 16; i++) {
        float t = (float(i) + 0.5) / 16.0;
        float angle = float(i) * 2.39996;
        vec2 offset = u_blur * sqrt(t) * vec2(cos(angle), sin(angle));
        float weight = exp(-2.0 * t);
        sum += textureLod(u_backdrop, (q + offset) / u_texture_size, lod).rgb * weight;
        total += weight;
    }
    vec3 color = sum / total;
    float gray = dot(color, vec3(0.299, 0.587, 0.114));
    color = clamp(mix(vec3(gray), color, 1.2), 0.0, 1.0);
    frag_color = vec4(color * coverage, coverage);
}
";

/// Compile the lens program and create its texture.
///
/// # Safety
/// Call with the context current on this thread.
unsafe fn create(gl: &glow::Context) -> Result<Resources, String> {
    let version = egui_glow::ShaderVersion::get(gl);
    if !version.is_new_shader_interface() {
        return Err(format!("{version:?} shaders cannot read mip levels"));
    }
    let header = format!(
        "{}{}",
        version.version_declaration(),
        if version.is_embedded() {
            "precision highp float;\n"
        } else {
            ""
        }
    );
    // SAFETY: the caller guarantees a current context.
    unsafe {
        let program = gl.create_program()?;
        let mut shaders = Vec::new();
        for (kind, source) in [
            (glow::VERTEX_SHADER, VERTEX),
            (glow::FRAGMENT_SHADER, FRAGMENT),
        ] {
            let shader = gl.create_shader(kind)?;
            gl.shader_source(shader, &format!("{header}{source}"));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                let log = gl.get_shader_info_log(shader);
                gl.delete_shader(shader);
                for shader in shaders {
                    gl.delete_shader(shader);
                }
                gl.delete_program(program);
                return Err(log);
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        gl.link_program(program);
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        if !gl.get_program_link_status(program) {
            let log = gl.get_program_info_log(program);
            gl.delete_program(program);
            return Err(log);
        }
        let vertex_array = gl.create_vertex_array()?;
        let texture = gl.create_texture()?;
        gl.bind_texture(glow::TEXTURE_2D, Some(texture));
        for (parameter, value) in [
            (glow::TEXTURE_MIN_FILTER, glow::LINEAR_MIPMAP_LINEAR),
            (glow::TEXTURE_MAG_FILTER, glow::LINEAR),
            (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
            (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
        ] {
            gl.tex_parameter_i32(glow::TEXTURE_2D, parameter, value as i32);
        }
        gl.bind_texture(glow::TEXTURE_2D, None);
        let uniform = |name| gl.get_uniform_location(program, name);
        Ok(Resources {
            program,
            vertex_array,
            texture,
            size: [0, 0],
            backdrop: uniform("u_backdrop"),
            texture_size: uniform("u_texture_size"),
            rect: uniform("u_rect"),
            radii: uniform("u_radii"),
            blur: uniform("u_blur"),
            refraction: uniform("u_refraction"),
            fade: uniform("u_fade"),
        })
    }
}
