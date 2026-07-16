use super::*;


fn dpi_unit(bits: f32, entropy: f32) -> f32 {
    1.0 - 2.0_f32.powf(-2.0 * bits / entropy)
}
