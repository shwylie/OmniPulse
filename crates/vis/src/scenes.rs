// The scenes: smoke from the cover, a swarm of stars and marbled ink, each
// made of long-public techniques. Each is GLSL passes (see lib.rs for what they're given). Written to
// run as they are in WebGL2 too: no arrays of constants, no names GL's ES keeps for itself.

use crate::{Input, Kind, Pass, Scene, Size};

const NONE: Input = Input::None;
const fn p(i: usize) -> Input { Input::Pass(i) }

/// Shared by the scenes: a hash, smooth noise, a palette (Inigo Quilez's cosine one, public), rotations
const COMMON: &str = r#"
const float PI = 3.14159265;
float hash11(float n) { return fract(sin(n * 127.1) * 43758.5453); }
vec3 hash31(float n) { return fract(sin(vec3(n, n + 1.7, n + 3.1) * vec3(127.1, 311.7, 74.7)) * 43758.5453); }
float hash21(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }
float vnoise(vec2 p) {
    vec2 i = floor(p); vec2 f = fract(p); vec2 w = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash21(i), hash21(i + vec2(1.0, 0.0)), w.x), mix(hash21(i + vec2(0.0, 1.0)), hash21(i + vec2(1.0, 1.0)), w.x), w.y);
}
float fbm(vec2 p) { float a = 0.5; float s = 0.0; for (int i = 0; i < 5; i++) { s += a * vnoise(p); p = p * 2.03 + vec2(1.7, 9.2); a *= 0.5; } return s; }
vec3 palette(float t, vec3 a, vec3 b, vec3 c, vec3 d) { return a + b * cos(6.28318 * (c * t + d)); }
mat2 rot(float a) { float c = cos(a); float s = sin(a); return mat2(c, -s, s, c); }
vec3 tonemap(vec3 c) { c = max(c, vec3(0.0)); return c / (1.0 + c) * 1.15; }
"#;

// ---------- Swarm: 65,536 points resting as a shape, scattered by hits, gathering again ----------

/// (where point `id` belongs: on a sphere, a ring, a spiral galaxy or a knot, the shape stepping on the beat)
const SWARM_SHAPES: &str = r#"
vec3 shape(float id, float n, float which) {
    float u = id / n;
    vec3 h = hash31(id);
    float k = mod(which, 4.0);
    if (k < 1.0) {
        float phi = acos(1.0 - 2.0 * u); float th = PI * (1.0 + sqrt(5.0)) * id;
        return vec3(sin(phi) * cos(th), cos(phi), sin(phi) * sin(th)) * (0.95 + 0.08 * h.x);
    } else if (k < 2.0) {
        float a = u * 2.0 * PI * 40.0; float b = h.x * 2.0 * PI;
        return vec3((1.0 + 0.32 * cos(b)) * cos(a / 40.0), 0.32 * sin(b), (1.0 + 0.32 * cos(b)) * sin(a / 40.0));
    } else if (k < 3.0) {
        float arm = floor(h.x * 3.0); float r = pow(h.y, 0.7) * 1.4; float a = r * 3.5 + arm * 2.094 + h.z * 0.5;
        return vec3(cos(a) * r, (h.z - 0.5) * 0.12 * (1.5 - r), sin(a) * r);
    }
    float t = u * 2.0 * PI * 3.0; float s = h.x * 2.0 * PI;
    vec3 c = vec3(sin(t) + 2.0 * sin(2.0 * t), cos(t) - 2.0 * cos(2.0 * t), -sin(3.0 * t)) * 0.33;
    return c + 0.09 * vec3(cos(s), sin(s), cos(s * 1.3));
}
"#;

const SWARM_VEL: &str = concat!(r#"
// pass 0, velocity (xyz): towards its place, stirred by the music, flung out on hits
void mainImage(out vec4 o, in vec2 fc) {
    ivec2 ip = ivec2(fc);
    float n = chRes(0).x * chRes(0).y;
    float id = float(ip.y) * chRes(0).x + float(ip.x);
    vec3 pos = fetch0(ip).xyz;
    vec3 vel = fetch1(ip).xyz;
    if (iFrame < 2) { o = vec4(0.0); return; }
    vec3 target = shape(id, n, BEAT / 8.0 + floor(HITS / 64.0) * 0.0);
    float dt = clamp(iTimeDelta, 0.001, 0.05);
    vec3 f = (target - pos) * (6.0 - 4.0 * BUILD);
    // (a swirl: the music's level turns it)
    vec3 h = hash31(id) - 0.5;
    f += vec3(-pos.z, 0.0, pos.x) * (0.6 + 2.5 * LEVEL) + h * (1.5 + 6.0 * HIGH);
    // (a kick flings each out from the middle; a drop, much further)
    float kick = KICK * KICK;
    f += normalize(pos + h * 0.01) * (kick * 14.0 + DROP * 32.0) * (0.4 + hash11(id + floor(BEAT)));
    vel = vel * exp(-dt * 3.0) + f * dt;
    o = vec4(vel, 1.0);
}
"#);

const SWARM_POS: &str = r#"
// pass 1, position (xyz): moved by this frame's velocity
void mainImage(out vec4 o, in vec2 fc) {
    ivec2 ip = ivec2(fc);
    float n = chRes(0).x * chRes(0).y;
    float id = float(ip.y) * chRes(0).x + float(ip.x);
    if (iFrame < 2) { o = vec4((hash31(id) - 0.5) * 3.0, 1.0); return; }
    vec3 pos = fetch0(ip).xyz;
    vec3 vel = fetch1(ip).xyz;
    o = vec4(pos + vel * clamp(iTimeDelta, 0.001, 0.05), 1.0);
}
"#;

const SWARM_POINTS: &str = r#"
// pass 2, the points: seen by a camera going slowly round, nearer ones bigger, coloured by their speed
vec4 point(int id, out vec4 col, out float size) {
    vec2 r = chRes(0);
    ivec2 ip = ivec2(id % int(r.x), id / int(r.x));
    vec3 pos = fetch0(ip).xyz;
    vec3 vel = fetch1(ip).xyz;
    float a = iTime * 0.1;
    pos.xz = rot(a) * pos.xz;
    pos.yz = rot(0.35) * pos.yz;
    float z = pos.z + 3.2;
    float aspect = u.res.x / u.res.y;
    vec4 clip = vec4(pos.x * 1.7 / aspect, pos.y * 1.7, 0.5, z);
    float sp = length(vel);
    vec3 h = hash31(float(id) * 1.37);
    // (a star's own colour: most blue-white, then gold, rose and violet; the music turning them a little)
    vec3 star = h.x < 0.45 ? vec3(0.65, 0.8, 1.0) : h.x < 0.65 ? vec3(1.0, 0.82, 0.45) : h.x < 0.82 ? vec3(1.0, 0.45, 0.6) : h.x < 0.94 ? vec3(0.7, 0.5, 1.0) : vec3(0.4, 1.0, 0.9);
    star = mix(star, palette(h.y + MID * 0.4 + iTime * 0.02, vec3(0.5), vec3(0.5), vec3(1.0), vec3(0.0, 0.33, 0.67)), 0.35);
    float twinkle = 0.6 + 0.4 * sin(iTime * (2.0 + 5.0 * h.z) + h.y * 30.0);
    col = vec4(star * (0.045 + 0.03 * sp) * twinkle * (2.2 / z) * (1.0 + 1.2 * HIGH), 1.0);
    size = (0.9 + 1.6 * h.z * h.z) * (3.0 / z) * (1.0 + 0.5 * KICK);
    return clip;
}
"#;

const SWARM_TRAIL: &str = r#"
// pass 3: the points over what was, fading (a little trail behind each)
void mainImage(out vec4 o, in vec2 fc) {
    vec2 uv = fc / iResolution.xy;
    vec3 now = ch0(uv).rgb;
    vec3 was = ch1(uv).rgb;
    o = vec4(now + was * (0.72 + 0.2 * BUILD), 1.0);
}
"#;

const SWARM_IMAGE: &str = r#"
void mainImage(out vec4 o, in vec2 fc) {
    vec2 uv = fc / iResolution.xy;
    vec3 c = ch0(uv).rgb;
    // (each star's glow: near and far, wide and soft)
    vec3 b = vec3(0.0);
    for (int i = 1; i <= 4; i++) {
        vec2 px = float(i * i) * 1.5 / chRes(0);
        b += (ch0(uv + px).rgb + ch0(uv - px).rgb + ch0(uv + vec2(px.x, -px.y)).rgb + ch0(uv - vec2(px.x, -px.y)).rgb) * (0.5 / float(i));
    }
    c += b * 0.55;
    // (a faint nebula behind them, its colours drifting)
    vec2 p = (fc - 0.5 * iResolution.xy) / iResolution.y;
    vec3 neb = palette(fbm(p * 2.0 + iTime * 0.01) + 0.3, vec3(0.5), vec3(0.5), vec3(1.0), vec3(0.2, 0.4, 0.7)) * 0.03 * fbm(p * 3.0 - iTime * 0.02);
    vec3 bg = vec3(0.006, 0.008, 0.016) * (1.0 + 2.0 * DROP) + neb * (1.0 + LEVEL);
    o = vec4(tonemap(c * 2.4 + bg), 1.0);
}
"#;

// ---------- Ink: marbled ink moved by a flow, stirred by kicks ----------

const INK_FLOW: &str = r#"
// pass 0, the flow (xy, in the picture's units per second): a slow curl of noise, kicks dropping swirls
vec2 curl(vec2 p) { float e = 0.01; float n1 = fbm(p + vec2(0.0, e)); float n2 = fbm(p - vec2(0.0, e)); float n3 = fbm(p + vec2(e, 0.0)); float n4 = fbm(p - vec2(e, 0.0)); return vec2(n1 - n2, n4 - n3) / (2.0 * e); }
void mainImage(out vec4 o, in vec2 fc) {
    vec2 uv = fc / iResolution.xy;
    vec2 p = (fc - 0.5 * iResolution.xy) / iResolution.y;
    vec2 v = curl(p * 1.6 + iTime * 0.012) * (0.008 + 0.022 * MID + 0.01 * LEVEL);
    // (a swirl where this beat's kick lands, turning one way then the other)
    vec2 c = (hash31(floor(BEAT)).xy - 0.5) * vec2(1.4, 0.8);
    vec2 d = p - c;
    float s = exp(-dot(d, d) * 9.0) * KICK * (mod(BEAT, 2.0) < 1.0 ? 1.0 : -1.0);
    v += vec2(-d.y, d.x) * s * 0.45;
    vec2 last = ch0(uv).xy;
    o = vec4(mix(v, last, iFrame > 0 ? 0.6 : 0.0), 0.0, 1.0);
}
"#;

const INK_DYE: &str = r#"
// pass 1, the ink (rgb): carried along the flow; new colour dropped in with the spectrum
void mainImage(out vec4 o, in vec2 fc) {
    vec2 uv = fc / iResolution.xy;
    vec2 aspect = vec2(iResolution.y / iResolution.x, 1.0);
    vec2 v = ch0(uv).xy;
    float dt = clamp(iTimeDelta, 0.001, 0.05);
    vec3 was = ch1(uv - v * aspect * dt).rgb;
    // (its paints: any colour on the wheel, the wheel itself turning slowly as the song goes so its colours keep
    // changing; and a steady share of deep, near-black paint between them, so the contrast stays the same whatever
    // the colours are)
    #define INK(t) (palette(t, vec3(0.5), vec3(0.5), vec3(1.0, 1.0, 1.0), vec3(0.0, 0.33, 0.67) + iTime * vec3(0.011, 0.017, 0.007)) * mix(0.07, 1.0, smoothstep(0.15, 0.55, abs(fract((t) * 1.7) - 0.5) * 2.0)))
    if (iFrame < 2) {
        vec2 p = uv * vec2(iResolution.x / iResolution.y, 1.0) * 2.5;
        was = INK(fbm(p + fbm(p * 1.7)) * 1.3);
    }
    // (new paint poured in: along the spectrum's loud parts at the foot, and a pour on each kick and snare, each
    // a colour of its own, round the paints as the song goes)
    float sp = spectrum(uv.x);
    vec3 tint = INK(uv.x * 0.5 + iTime * 0.02 + MID * 0.3);
    float seep = smoothstep(0.12 * sp, 0.0, uv.y) * sp;
    vec2 bc = hash31(floor(float(HITS))).xy;
    vec2 bd = (uv - bc) / aspect;
    float blot = exp(-dot(bd, bd) * 90.0) * max(SNARE, KICK * 0.8);
    vec3 pour = INK(hash11(floor(float(HITS)) * 3.1) + iTime * 0.01);
    vec3 c = mix(was, tint, clamp(seep * 0.3, 0.0, 1.0));
    c = mix(c, pour, clamp(blot, 0.0, 1.0));
    o = vec4(c, 1.0);
}
"#;

const INK_IMAGE: &str = r#"
void mainImage(out vec4 o, in vec2 fc) {
    vec2 uv = fc / iResolution.xy;
    vec2 px = 1.5 / chRes(1);
    vec3 c = ch1(uv).rgb;
    // (lit as a thin wet sheet: its slope from the ink's brightness, a light from the top left)
    float hx = dot(ch1(uv + vec2(px.x, 0.0)).rgb - ch1(uv - vec2(px.x, 0.0)).rgb, vec3(0.33));
    float hy = dot(ch1(uv + vec2(0.0, px.y)).rgb - ch1(uv - vec2(0.0, px.y)).rgb, vec3(0.33));
    vec3 n = normalize(vec3(-hx * 6.0, -hy * 6.0, 1.0));
    float spec = pow(clamp(dot(reflect(normalize(vec3(-0.4, -0.5, -1.0)), n), vec3(0.0, 0.0, 1.0)), 0.0, 1.0), 24.0);
    c = c * (0.85 + 0.3 * n.z) + spec * (0.25 + 0.6 * HIGH);
    // (gold veins where one paint meets another, catching the light)
    float edge = clamp(length(vec2(hx, hy)) * 7.0, 0.0, 1.0);
    vec3 gold = vec3(1.0, 0.78, 0.38) * (0.7 + 0.6 * spec + 0.5 * HIGH);
    c = mix(c, gold, smoothstep(0.55, 0.95, edge) * 0.75);
    // (deep and rich as wet paint: its colours fuller; and its brightness levelled to one fixed range, from deep to
    // bright, so the contrast's the same whatever colours are in it)
    float l = max(dot(c, vec3(0.299, 0.587, 0.114)), 1e-4);
    c = mix(vec3(l), c, 1.4);
    float want = mix(0.03, 0.78, smoothstep(0.04, 0.6, l));
    c = max(c, vec3(0.0)) * (want / l);
    o = vec4(tonemap(c * (1.25 + 0.3 * KICK)), 1.0);
}
"#;

// ---------- Smoke: the song's cover, drifting off as smoke with the music and forming again ----------

const SMOKE_FLOW: &str = r#"
// pass 0, the draft (xy): a slow curl that the level speeds up; each kick a puff outward from the middle
vec2 curl(vec2 p) { float e = 0.02; float a = fbm(p + vec2(0.0, e)); float b = fbm(p - vec2(0.0, e)); float c = fbm(p + vec2(e, 0.0)); float d = fbm(p - vec2(e, 0.0)); return vec2(a - b, d - c) / (2.0 * e); }
void mainImage(out vec4 o, in vec2 fc) {
    vec2 uv = fc / iResolution.xy;
    vec2 p = (fc - 0.5 * iResolution.xy) / iResolution.y;
    vec2 v = curl(p * 2.2 + vec2(0.0, -iTime * 0.08)) * (0.03 + 0.1 * LEVEL + 0.07 * MID + 0.04 * HIGH);
    // (smoke rises, a little faster with the music)
    v.y += 0.03 + 0.05 * LEVEL;
    // (each kick a puff out from somewhere new; a snare a flick sideways; a drop, everything blown)
    vec2 at = (hash31(floor(BEAT)).xy - 0.5) * vec2(1.2, 0.7);
    vec2 d = p - at;
    v += normalize(d + 1e-4) * (0.6 * KICK * KICK) * exp(-dot(d, d) * 3.0);
    v.x += SNARE * 0.08 * (mod(float(HITS), 2.0) < 1.0 ? 1.0 : -1.0) * exp(-dot(p, p));
    v += normalize(p + 1e-4) * 0.9 * DROP * exp(-dot(p, p) * 1.2);
    vec2 last = ch0(uv).xy;
    o = vec4(mix(v, last, iFrame > 0 ? 0.5 : 0.0), 0.0, 1.0);
}
"#;

/// (the cover, whole, as a square in the middle of the dark (its edges soft), the smoke drifting off it)
const SMOKE_ART: &str = r#"
vec3 cover(vec2 uv, float zoom) {
    vec2 res = iResolution.xy;
    vec2 q = (uv * res - 0.5 * res) / (max(res.x, res.y) * zoom) + 0.5;
    if (chRes(2).x < 2.0) return vec3(0.12, 0.1, 0.16) * (1.0 - length(uv - 0.5));
    return ch2(clamp(q, 0.0, 1.0)).rgb;
}
"#;

const SMOKE_BODY: &str = r#"
// pass 1, the smoke (rgb, and how much is smoke in a): carried along the draft, fading back to the cover
void mainImage(out vec4 o, in vec2 fc) {
    vec2 uv = fc / iResolution.xy;
    vec2 aspect = vec2(iResolution.y / iResolution.x, 1.0);
    vec2 v = ch0(uv).xy;
    float dt = clamp(iTimeDelta, 0.001, 0.05);
    // (breathing with the bass: the cover a little nearer on each kick)
    vec3 art = cover(uv, 1.0 + 0.012 * BASS + 0.015 * KICK);
    if (iFrame < 2) { o = vec4(art, 0.0); return; }
    vec4 was = ch1(uv - v * aspect * dt);
    // (wisps: where the noise says, a little of the smoke thins and lightens as it goes)
    float w = fbm(uv * vec2(iResolution.x / iResolution.y, 1.0) * 4.0 - v * 6.0 + iTime * 0.2);
    // (only a touch each frame: the smoke keeps the cover's colours, paler at its thin edges)
    vec3 smoke = mix(was.rgb, vec3(dot(was.rgb, vec3(0.33))) * 1.05, (0.004 + 0.012 * w) * was.a);
    // (how fast the cover comes back: quickly when it's quiet, slower as it builds, hardly at a drop)
    float back = clamp(dt * (3.0 - 2.2 * LEVEL - 1.2 * BUILD - 2.5 * DROP), 0.006, 0.09);
    vec3 c = mix(smoke, art, back);
    float amount = clamp(was.a * (1.0 - back * 2.0) + length(v) * dt * 6.0, 0.0, 1.0);
    o = vec4(c, amount);
}
"#;

const SMOKE_IMAGE: &str = r#"
void mainImage(out vec4 o, in vec2 fc) {
    vec2 uv = fc / iResolution.xy;
    vec4 s = ch1(uv);
    vec3 art = cover(uv, 1.0);
    // (a soft glow off the smoke, brighter on the beat; the edges darkened)
    vec2 px = 3.0 / chRes(1);
    vec3 b = (ch1(uv + vec2(px.x, 0.0)).rgb + ch1(uv - vec2(px.x, 0.0)).rgb + ch1(uv + vec2(0.0, px.y)).rgb + ch1(uv - vec2(0.0, px.y)).rgb) * 0.25;
    // (a steady glow: never brighter on a kick, so nothing flickers as the music's level changes)
    vec3 c = s.rgb + max(b - 0.35, vec3(0.0)) * 0.55 * s.a;
    c = mix(c, art, 0.08 * (1.0 - s.a));
    float vig = smoothstep(1.4, 0.4, length((uv - 0.5) * vec2(iResolution.x / iResolution.y, 1.0)) * 0.9);
    o = vec4(tonemap(c * vig * 1.1) + (hash21(fc + iTime) - 0.5) * 0.01, 1.0);
}
"#;

fn pass(kind: Kind, size: Size, inputs: [Input; 4], src: &'static str) -> Pass { Pass { kind, size, inputs, src } }

/// Every scene: its id (kept in the settings), its name, its passes
pub fn all() -> &'static [Scene] {
    static SCENES: std::sync::OnceLock<Vec<Scene>> = std::sync::OnceLock::new();
    SCENES.get_or_init(make)
}

fn make() -> Vec<Scene> {
    // (each pass's source: what's shared, then its own; made once, as the scenes last as long as the app)
    let with = |parts: &[&str]| -> &'static str { Box::leak(parts.concat().into_boxed_str()) };
    vec![
        Scene { id: "smoke", name: "Smoke", passes: vec![
            pass(Kind::Buffer, Size::Scaled(0.25), [p(0), NONE, NONE, NONE], with(&[COMMON, SMOKE_FLOW])),
            pass(Kind::Buffer, Size::Scaled(0.75), [p(0), p(1), Input::Art, NONE], with(&[COMMON, SMOKE_ART, SMOKE_BODY])),
            pass(Kind::Image, Size::Scaled(1.0), [NONE, p(1), Input::Art, NONE], with(&[COMMON, SMOKE_ART, SMOKE_IMAGE])),
        ] },
        Scene { id: "swarm", name: "Swarm", passes: vec![
            pass(Kind::Buffer, Size::Fixed(256, 256), [p(1), p(0), NONE, NONE], with(&[COMMON, SWARM_SHAPES, SWARM_VEL])),
            pass(Kind::Buffer, Size::Fixed(256, 256), [p(1), p(0), NONE, NONE], with(&[COMMON, SWARM_POS])),
            pass(Kind::Points(256 * 256), Size::Scaled(0.75), [p(1), p(0), NONE, NONE], with(&[COMMON, SWARM_POINTS])),
            pass(Kind::Buffer, Size::Scaled(0.75), [p(2), p(3), NONE, NONE], with(&[COMMON, SWARM_TRAIL])),
            pass(Kind::Image, Size::Scaled(1.0), [p(3), NONE, NONE, NONE], with(&[COMMON, SWARM_IMAGE])),
        ] },
        Scene { id: "ink", name: "Ink", passes: vec![
            pass(Kind::Buffer, Size::Scaled(0.25), [p(0), NONE, NONE, NONE], with(&[COMMON, INK_FLOW])),
            pass(Kind::Buffer, Size::Scaled(0.6), [p(0), p(1), NONE, NONE], with(&[COMMON, INK_DYE])),
            pass(Kind::Image, Size::Scaled(1.0), [NONE, p(1), NONE, NONE], with(&[COMMON, INK_IMAGE])),
        ] },
    ]
}

pub fn by_id(id: &str) -> Option<&'static Scene> { all().iter().find(|s| s.id == id) }
