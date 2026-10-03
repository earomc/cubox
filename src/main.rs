use std::{collections::HashSet, time::Instant};

use steel_worldgen::biomes::{BiomeSourceKind};

fn main() {
    let seed = 3699507159329829894;

    let biome_source = BiomeSourceKind::overworld(seed);
    const SIZE: u32 = 1024;
    let duration = Instant::now();
    let mut imgbuf = image::ImageBuffer::new(SIZE, SIZE);
    //let mut set = HashSet::new();
    let mut sampler = biome_source.chunk_sampler();
    for x in 0..SIZE {
        for z in 0..SIZE {
            let (quart_x, quart_z): (i32, i32) = (
                (x as i32) - SIZE as i32 / 2,
                (z as i32) - SIZE as i32 / 2,
            );
            
            let biome = sampler.sample(quart_x, 60, quart_z);
            //set.insert(biome.key.path.to_owned());
            /*
            match &*biome.key.path {
                "river" | "ocean" | "deep_ocean" | "lukewarm_ocean" | "deep_lukewarm_ocean" => {
                    print!(" ")
                }
                _ => print!("X"),
            }
             */
            let pixel = imgbuf.get_pixel_mut(x, z);
            match &*biome.key.path {
                "river" | "ocean" | "deep_ocean" | "lukewarm_ocean" | "deep_lukewarm_ocean" => {
                    *pixel = image::Rgb([0_u8, 0_u8, 127_u8]);
                }
                _ => {
                    *pixel = image::Rgb([0_u8, 127_u8, 0_u8]);
                }
            }
        }
    }
    let time_generation = duration.elapsed();
    println!("generated in {:?}", time_generation);
    imgbuf.save("map.png").unwrap();
    let time_saved = duration.elapsed();
    println!("image saved in {:?}", time_saved);
}
