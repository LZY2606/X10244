use saxs_station::domain;

fn main() {
    println!("saxs-station {}", env!("CARGO_PKG_VERSION"));
    let _ = domain::q_unit_to_inv_a("1/A");
}
