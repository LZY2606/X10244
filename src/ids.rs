//! 稳定身份与内容指纹。所有 id 由结构化键决定，便于导出/删除/再导入后保持一致。

use sha_lite::fnv1a64;

pub fn node_normalized(material_id: &str) -> String {
    format!("norm/{}", material_id)
}
pub fn node_subtraction(sample_mat: &str, buffer_mat: &str) -> String {
    format!("sub/{}__{}", sample_mat, buffer_mat)
}
pub fn node_model(sub_node: &str) -> String {
    format!("model/{}", sub_node.trim_start_matches("sub/"))
}
pub fn version(node_id: &str, ordinal: u32) -> String {
    format!("{}/v{}", node_id, ordinal)
}
pub fn material_version(id: &str, rev: u32) -> String {
    format!("mat/{}/r{}", id, rev)
}
pub fn point(version_id: &str, i: usize) -> String {
    format!("{}/p{}", version_id, i)
}

/// 规范化 JSON 的内容指纹（仅用于导入校验，身份本身仍由结构化 id 决定）。
pub fn content_hash(canonical_json: &str) -> String {
    format!("{:016x}", fnv1a64(canonical_json.as_bytes()))
}
