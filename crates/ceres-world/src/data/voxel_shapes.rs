//! VoxelShapes built from voxel_shapes.json.

use super::*;

/// Port of StaticPacketCache::loadVoxelShapesFromFile().
pub(crate) fn encode_voxel_shapes(path: &Path) -> Res<Vec<u8>> {
    let data: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path).map_err(|e| format!("voxel_shapes.json: {e}"))?)
            .map_err(|e| e.to_string())?;
    let shapes = data.as_array().ok_or("voxel_shapes.json is not a list")?;
    let mut body = W::new();
    let mut names: Vec<(String, u16)> = Vec::new();
    body.uvarint(shapes.len() as u32);
    for (i, shape) in shapes.iter().enumerate() {
        if let Some(n) = shape["identifier"].as_str() {
            names.retain(|(k, _)| k != n);
            names.push((n.to_string(), i as u16));
        }
        let boxes: Vec<[[f64; 3]; 2]> = shape["boxes"]
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|b| {
                        let p = |j: usize, k: usize| b[j][k].as_f64().unwrap_or(0.0) / 16.0;
                        [[p(0, 0), p(0, 1), p(0, 2)], [p(1, 0), p(1, 1), p(1, 2)]]
                    })
                    .collect()
            })
            .unwrap_or_default();
        if boxes.is_empty() {
            body.bytes(&[0, 0, 0]);
            body.uvarint(0);
            for _ in 0..3 {
                body.uvarint(1);
                body.f32_le(0.0);
            }
            continue;
        }
        let axis = |a: usize| -> Vec<f64> {
            let mut v: Vec<f64> = boxes.iter().flat_map(|b| [b[0][a], b[1][a]]).collect();
            v.sort_by(|x, y| x.partial_cmp(y).unwrap());
            v.dedup();
            v
        };
        let (xs, ys, zs) = (axis(0), axis(1), axis(2));
        let (rx, ry, rz) = (xs.len() - 1, ys.len() - 1, zs.len() - 1);
        let mut storage = vec![0u8; (rx * ry * rz).div_ceil(8)];
        for z in 0..rz {
            for y in 0..ry {
                for x in 0..rx {
                    let m = [(xs[x] + xs[x + 1]) / 2.0, (ys[y] + ys[y + 1]) / 2.0, (zs[z] + zs[z + 1]) / 2.0];
                    if boxes.iter().any(|b| (0..3).all(|a| m[a] >= b[0][a] && m[a] <= b[1][a])) {
                        let index = z + y * rz + x * rz * ry;
                        storage[index / 8] |= 1 << (index % 8);
                    }
                }
            }
        }
        body.u8(rx as u8);
        body.u8(ry as u8);
        body.u8(rz as u8);
        body.uvarint(storage.len() as u32);
        body.bytes(&storage);
        for coords in [&xs, &ys, &zs] {
            body.uvarint(coords.len() as u32);
            for c in coords.iter() {
                body.f32_le(*c as f32);
            }
        }
    }
    Ok(packet(id::VOXEL_SHAPES, |w| {
        w.bytes(&body.0);
        w.uvarint(names.len() as u32);
        for (n, i) in &names {
            w.string(n);
            w.u16_le(*i);
        }
        w.u16_le(0); // custom shape count
    }))
}
