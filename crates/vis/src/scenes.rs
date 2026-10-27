// The scenes: smoke from the cover, a fractal flight, a swarm of points, marbled ink and phosphor traces, each
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

// ---------- Depths: a flight round a Mandelbulb that regrows on the beat ----------

const DEPTHS_MARCH: &str = concat!(r#"
// (the bulb's power and twist step on each kick, eased between, so its form regrows with the beat)
float power() { return 7.0 + 2.5 * sin(BEAT * 0.83 + iTime * 0.05) + 0.6 * KICK; }
float bulb(vec3 p, out float trap) {
    vec3 z = p; float dr = 1.0; float r = 0.0; float pw = power(); trap = 1e9;
    for (int i = 0; i < 9; i++) {
        r = length(z);
        if (r > 2.0) break;
        float th = acos(clamp(z.z / r, -1.0, 1.0)) * pw + iTime * 0.07;
        float ph = atan(z.y, z.x) * pw;
        dr = pow(r, pw - 1.0) * pw * dr + 1.0;
        z = pow(r, pw) * vec3(sin(th) * cos(ph), sin(ph) * sin(th), cos(th)) + p;
        trap = min(trap, dot(z, z));
    }
    return 0.5 * log(r) * r / dr;
}
float map(vec3 p) { float t; return bulb(p, t); }
vec3 normal(vec3 p, float e) { vec2 k = vec2(1.0, -1.0); return normalize(k.xyy * map(p + k.xyy * e) + k.yyx * map(p + k.yyx * e) + k.yxy * map(p + k.yxy * e) + k.xxx * map(p + k.xxx * e)); }
void mainImage(out vec4 o, in vec2 fc) {
    vec2 uv = (fc - 0.5 * iResolution.xy) / iResolution.y;
    float t = iTime * 0.12;
    // (the camera: round and in and out, pushed in a little on each kick)
    float dist = 3.3 - 0.9 * smoothstep(0.0, 1.0, 0.5 + 0.5 * sin(t * 0.7)) - 0.15 * KICK - 0.2 * DROP;
    vec3 ro = vec3(dist * cos(t), 0.6 * sin(t * 0.6), dist * sin(t));
    vec3 fw = normalize(-ro); vec3 rt = normalize(cross(vec3(0.0, 1.0, 0.0), fw)); vec3 up = cross(fw, rt);
    vec3 rd = normalize(fw * (1.4 + 0.3 * BUILD) + uv.x * rt + uv.y * up);
    float d = 0.0; float trap = 0.0; float steps = 0.0; bool hit = false;
    for (int i = 0; i < 110; i++) {
        vec3 q = ro + rd * d;
        float h = bulb(q, trap);
        if (h < 0.0007 * d) { hit = true; break; }
        d += h * 0.9;
        steps += 1.0;
        if (d > 7.0) break;
    }
    vec3 bg = palette(0.6 + uv.y * 0.2 + MID * 0.2, vec3(0.03), vec3(0.04), vec3(1.0), vec3(0.6, 0.7, 0.8));
    vec3 col = bg;
    vec3 tint = palette(trap * 0.35 + MID * 0.6 + iTime * 0.01, vec3(0.5), vec3(0.5), vec3(1.0, 1.0, 1.0), vec3(0.0, 0.15, 0.3));
    if (hit) {
        vec3 pos = ro + rd * d;
        vec3 n = normal(pos, 0.0005 * d);
        vec3 l = normalize(vec3(0.6, 0.8, -0.4));
        float dif = clamp(dot(n, l), 0.0, 1.0);
        float rim = pow(1.0 - clamp(dot(n, -rd), 0.0, 1.0), 3.0);
        float ao = clamp(1.0 - steps / 110.0, 0.0, 1.0);
        col = tint * (0.15 + 0.9 * dif) * ao + rim * (0.4 + 1.2 * HIGH) * palette(0.1 + HIGH, vec3(0.6), vec3(0.4), vec3(1.0), vec3(0.2, 0.4, 0.6));
        col *= 1.0 + 0.8 * KICK + 0.8 * DROP;
    }
    // (a glow where the march came close: the bulb's own light, brighter with the level)
    col += tint * pow(steps / 110.0, 2.0) * (0.4 + 1.5 * LEVEL);
    // (last frame's, a little: smoother, a touch of trail)
    vec3 last = ch0(fc / iResolution.xy).rgb;
    o = vec4(mix(col, last, iFrame > 0 ? 0.35 : 0.0), 1.0);
}
"#);

const DEPTHS_IMAGE: &str = r#"
void mainImage(out vec4 o, in vec2 fc) {
    vec2 uv = fc / iResolution.xy;
    vec3 c = ch0(uv).rgb;
    // (a soft bloom from the picture, 4 taps out)
    vec2 px = 3.0 / chRes(0);
    vec3 b = (ch0(uv + vec2(px.x, 0.0)).rgb + ch0(uv - vec2(px.x, 0.0)).rgb + ch0(uv + vec2(0.0, px.y)).rgb + ch0(uv - vec2(0.0, px.y)).rgb) * 0.25;
    c += max(b - 0.5, vec3(0.0)) * (0.6 + KICK);
    float v = smoothstep(1.2, 0.3, length(uv - 0.5));
    o = vec4(tonemap(c * v * 1.3) + (hash21(fc + iTime) - 0.5) * 0.02, 1.0);
}
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
    col = vec4(palette(0.55 + sp * 0.08 + MID * 0.3, vec3(0.5), vec3(0.5), vec3(1.0), vec3(0.0, 0.1, 0.2)) * (0.018 + 0.012 * sp) * (2.2 / z), 1.0);
    size = 1.4 * (3.0 / z) * (1.0 + 0.4 * KICK);
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
    vec2 px = 2.0 / chRes(0);
    vec3 b = (ch0(uv + px).rgb + ch0(uv - px).rgb + ch0(uv + vec2(px.x, -px.y)).rgb + ch0(uv - vec2(px.x, -px.y)).rgb) * 0.25;
    c += b * 0.8;
    vec3 bg = vec3(0.01, 0.012, 0.02) * (1.0 + 2.0 * DROP);
    o = vec4(tonemap(c * 1.6 + bg), 1.0);
}
"#;

// ---------- Ink: marbled ink moved by a flow, stirred by kicks ----------

const INK_FLOW: &str = r#"
// pass 0, the flow (xy, in the picture's units per second): a slow curl of noise, kicks dropping swirls
vec2 curl(vec2 p) { float e = 0.01; float n1 = fbm(p + vec2(0.0, e)); float n2 = fbm(p - vec2(0.0, e)); float n3 = fbm(p + vec2(e, 0.0)); float n4 = fbm(p - vec2(e, 0.0)); return vec2(n1 - n2, n4 - n3) / (2.0 * e); }
void mainImage(out vec4 o, in vec2 fc) {
    vec2 uv = fc / iResolution.xy;
    vec2 p = (fc - 0.5 * iResolution.xy) / iResolution.y;
    vec2 v = curl(p * 1.6 + iTime * 0.03) * (0.025 + 0.08 * MID);
    // (a swirl where this beat's kick lands, turning one way then the other)
    vec2 c = (hash31(floor(BEAT)).xy - 0.5) * vec2(1.4, 0.8);
    vec2 d = p - c;
    float s = exp(-dot(d, d) * 9.0) * KICK * (mod(BEAT, 2.0) < 1.0 ? 1.0 : -1.0);
    v += vec2(-d.y, d.x) * s * 1.6;
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
    if (iFrame < 2) {
        vec2 p = uv * vec2(iResolution.x / iResolution.y, 1.0) * 3.0;
        was = palette(fbm(p) * 0.9, vec3(0.35, 0.3, 0.4), vec3(0.35, 0.3, 0.35), vec3(1.0, 0.9, 0.8), vec3(0.6, 0.7, 0.85)) * 0.7;
    }
    // (a band of the spectrum's colour seeping in along the bottom, and blots on each snare)
    float sp = spectrum(uv.x);
    vec3 tint = palette(uv.x * 0.35 + iTime * 0.015 + MID * 0.25, vec3(0.4, 0.35, 0.45), vec3(0.4, 0.35, 0.4), vec3(1.0, 0.9, 0.8), vec3(0.6, 0.7, 0.85));
    float seep = smoothstep(0.08 * sp, 0.0, uv.y) * sp;
    vec2 bc = hash31(floor(float(HITS))).xy;
    vec2 bd = (uv - bc) / aspect;
    float blot = exp(-dot(bd, bd) * 140.0) * SNARE;
    vec3 c = mix(was, tint, clamp(seep * 0.25 + blot, 0.0, 1.0));
    o = vec4(c * (1.0 - dt * 0.03), 1.0);
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
    o = vec4(tonemap(c * (1.2 + 0.4 * KICK)), 1.0);
}
"#;

// ---------- Phosphor: the stereo wave as a glowing trace, held on the screen as an old scope's ----------

const PHOS_POINTS: &str = r#"
// pass 0, the trace: points along the wave, left against right (Lissajous), or round a circle; a drop
// swaps which, kicks open it out
vec4 point(int id, out vec4 col, out float size) {
    float n = 16384.0;
    float t = float(id) / n;
    vec2 w = wave(t);
    float mode = mod(floor(BEAT / 32.0) + (DROP > 0.5 ? 1.0 : 0.0), 2.0);
    vec2 q;
    if (mode < 1.0) {
        q = rot(0.785398) * w * (0.7 + 0.3 * KICK);
    } else {
        float a = t * 2.0 * PI;
        float r = 0.55 + 0.35 * (w.x + w.y) * 0.5 + 0.1 * KICK;
        q = vec2(cos(a), sin(a)) * r;
    }
    float aspect = u.res.x / u.res.y;
    col = vec4(mix(vec3(0.25, 1.0, 0.5), vec3(0.35, 0.75, 1.0), clamp(WIDTH * 1.5, 0.0, 1.0)) * (0.04 + 0.12 * LEVEL), 1.0);
    size = 2.0 + 2.0 * KICK;
    return vec4(q.x / aspect * 0.85, q.y * 0.85, 0.5, 1.0);
}
"#;

const PHOS_KEEP: &str = r#"
// pass 1: the trace held, fading as a phosphor does (longer while it builds)
void mainImage(out vec4 o, in vec2 fc) {
    vec2 uv = fc / iResolution.xy;
    vec3 was = ch1(uv).rgb * (0.86 + 0.1 * BUILD);
    o = vec4(was + ch0(uv).rgb, 1.0);
}
"#;

const PHOS_IMAGE: &str = r#"
void mainImage(out vec4 o, in vec2 fc) {
    vec2 uv = fc / iResolution.xy;
    vec3 c = ch0(uv).rgb;
    vec3 b = vec3(0.0);
    for (int i = 1; i <= 4; i++) {
        vec2 d = float(i) * 2.5 / chRes(0);
        b += ch0(uv + vec2(d.x, 0.0)).rgb + ch0(uv - vec2(d.x, 0.0)).rgb + ch0(uv + vec2(0.0, d.y)).rgb + ch0(uv - vec2(0.0, d.y)).rgb;
    }
    c += b * 0.06 * (1.0 + KICK);
    // (the scope's glass: a faint grid and its curve darkening the corners)
    vec2 g = abs(fract(uv * vec2(10.0, 8.0)) - 0.5);
    float grid = smoothstep(0.49, 0.5, max(g.x, g.y)) * 0.025;
    float v = smoothstep(1.1, 0.35, length(uv - 0.5));
    o = vec4(tonemap((c + grid) * v * 1.4), 1.0);
}
"#;

// ---------- Smoke: the song's cover, drifting off as smoke with the music and forming again ----------

const SMOKE_FLOW: &str = r#"
// pass 0, the draft (xy): a slow curl that the level speeds up; each kick a puff outward from the middle
vec2 curl(vec2 p) { float e = 0.02; float a = fbm(p + vec2(0.0, e)); float b = fbm(p - vec2(0.0, e)); float c = fbm(p + vec2(e, 0.0)); float d = fbm(p - vec2(e, 0.0)); return vec2(a - b, d - c) / (2.0 * e); }
void mainImage(out vec4 o, in vec2 fc) {
    vec2 uv = fc / iResolution.xy;
    vec2 p = (fc - 0.5 * iResolution.xy) / iResolution.y;
    vec2 v = curl(p * 2.2 + vec2(0.0, -iTime * 0.08)) * (0.02 + 0.1 * LEVEL + 0.08 * MID);
    // (smoke rises)
    v.y += 0.03 + 0.05 * LEVEL;
    v += normalize(p + 1e-4) * (0.5 * KICK * KICK + 1.4 * DROP) * exp(-dot(p, p) * 1.5);
    vec2 last = ch0(uv).xy;
    o = vec4(mix(v, last, iFrame > 0 ? 0.5 : 0.0), 0.0, 1.0);
}
"#;

/// (the cover, whole, as a square in the middle of the dark (its edges soft), the smoke drifting off it)
const SMOKE_ART: &str = r#"
vec3 cover(vec2 uv, float zoom) {
    vec2 res = iResolution.xy;
    vec2 q = (uv * res - 0.5 * res) / (res.y * 0.84 * zoom) + 0.5;
    if (chRes(2).x < 2.0) return vec3(0.12, 0.1, 0.16) * (1.0 - length(uv - 0.5));
    vec2 e = smoothstep(vec2(0.0), vec2(0.015), q) * smoothstep(vec2(1.0), vec2(0.985), q);
    vec3 dark = ch2(vec2(0.5)).rgb * 0.05;
    return mix(dark, ch2(clamp(q, 0.0, 1.0)).rgb, e.x * e.y);
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
    vec3 art = cover(uv, 1.0 + 0.03 * BASS + 0.04 * KICK);
    if (iFrame < 2) { o = vec4(art, 0.0); return; }
    vec4 was = ch1(uv - v * aspect * dt);
    // (wisps: where the noise says, a little of the smoke thins and lightens as it goes)
    float w = fbm(uv * vec2(iResolution.x / iResolution.y, 1.0) * 4.0 - v * 6.0 + iTime * 0.2);
    // (only a touch each frame: the smoke keeps the cover's colours, paler at its thin edges)
    vec3 smoke = mix(was.rgb, vec3(dot(was.rgb, vec3(0.33))) * 1.05, (0.004 + 0.012 * w) * was.a);
    // (how fast the cover comes back: quickly when it's quiet, slower as it builds, hardly at a drop)
    float back = clamp(dt * (4.0 - 2.6 * LEVEL - 1.2 * BUILD - 3.5 * DROP), 0.008, 0.12);
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
    vec3 c = s.rgb + max(b - 0.35, vec3(0.0)) * (0.5 + 0.8 * KICK) * s.a;
    c = mix(c, art, 0.08 * (1.0 - s.a));
    float vig = smoothstep(1.15, 0.25, length((uv - 0.5) * vec2(iResolution.x / iResolution.y, 1.0)) * 0.9);
    o = vec4(tonemap(c * vig * (1.05 + 0.25 * KICK + 0.4 * DROP)) + (hash21(fc + iTime) - 0.5) * 0.015, 1.0);
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
        Scene { id: "depths", name: "Depths", passes: vec![
            pass(Kind::Buffer, Size::Scaled(0.75), [p(0), NONE, NONE, NONE], with(&[COMMON, DEPTHS_MARCH])),
            pass(Kind::Image, Size::Scaled(1.0), [p(0), NONE, NONE, NONE], with(&[COMMON, DEPTHS_IMAGE])),
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
        Scene { id: "phosphor", name: "Phosphor", passes: vec![
            pass(Kind::Points(16384), Size::Scaled(0.75), [NONE; 4], with(&[COMMON, PHOS_POINTS])),
            pass(Kind::Buffer, Size::Scaled(0.75), [p(0), p(1), NONE, NONE], with(&[COMMON, PHOS_KEEP])),
            pass(Kind::Image, Size::Scaled(1.0), [p(1), NONE, NONE, NONE], with(&[COMMON, PHOS_IMAGE])),
        ] },
    ]
}

pub fn by_id(id: &str) -> Option<&'static Scene> { all().iter().find(|s| s.id == id) }
