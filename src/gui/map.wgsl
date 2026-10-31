// The map: one instanced quad per tile (or part of a tile), drawn straight onto the surface.

struct Screen {
    size: vec2<f32>,  // in physical pixels
    encode_srgb: u32, // the surface isn't sRGB, so encode by hand
    _pad: u32,
};

@group(0) @binding(0) var<uniform> screen: Screen;
@group(1) @binding(0) var tile_tex: texture_2d<f32>;
@group(1) @binding(1) var tile_sampler: sampler;

struct TileIn {
    @location(0) rect: vec4<f32>, // min x, min y, max x, max y in physical pixels
    @location(1) uv: vec4<f32>,   // min u, min v, max u, max v
    @location(2) alpha: f32,
};

struct TileOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) alpha: f32,
};

@vertex
fn vs_tile(@builtin(vertex_index) vi: u32, tile: TileIn) -> TileOut {
    // triangle strip corners: (0,0) (1,0) (0,1) (1,1)
    let corner = vec2<f32>(f32(vi & 1u), f32(vi >> 1u));
    let px = mix(tile.rect.xy, tile.rect.zw, corner);
    var out: TileOut;
    out.pos = vec4<f32>(px.x / screen.size.x * 2.0 - 1.0, 1.0 - px.y / screen.size.y * 2.0, 0.0, 1.0);
    out.uv = mix(tile.uv.xy, tile.uv.zw, corner);
    out.alpha = tile.alpha;
    return out;
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let lo = c * 12.92;
    let hi = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(hi, lo, c <= vec3<f32>(0.0031308));
}

@fragment
fn fs_tile(in: TileOut) -> @location(0) vec4<f32> {
    // tiles are sRGB textures, so filtering happens in linear light
    let texel = textureSample(tile_tex, tile_sampler, in.uv);
    var c = texel.rgb;
    if screen.encode_srgb != 0u {
        c = linear_to_srgb(c);
    }
    let alpha = texel.a * in.alpha;
    return vec4<f32>(c * alpha, alpha); // premultiplied
}

// Darkens what isn't highlighted, only drawn while biomes are. The number of the biome at each pixel
// looks up how much in a table, so changing which biomes needs no new tiles.
@group(1) @binding(2) var mask_tex: texture_2d<f32>;
@group(1) @binding(3) var darkening_tex: texture_2d<f32>;

@fragment
fn fs_mask(in: TileOut) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(mask_tex));
    let texel = clamp(vec2<i32>(in.uv * vec2<f32>(size)), vec2<i32>(0), size - vec2<i32>(1));
    let number = i32(textureLoad(mask_tex, texel, 0).r * 255.0 + 0.5);
    let darkening = textureLoad(darkening_tex, vec2<i32>(number, 0), 0).r * in.alpha;
    return vec4<f32>(0.0, 0.0, 0.0, darkening); // premultiplied
}
