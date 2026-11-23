/// A parsed RPC parameter with its name and a T-SQL literal value string.
#[derive(Debug, Clone)]
pub struct RpcParam {
    pub name: String,
    pub type_name: String,
    pub value_sql: String,
    pub tvp_rows: Option<Vec<Vec<String>>>,
    pub is_output: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputParamBinding {
    pub ordinal: u16,
    pub param_name: String,
    pub var_name: String,
}

/// Result of parsing an RPC packet.
#[derive(Debug)]
pub struct RpcRequest {
    pub sql: String,
    pub params: Vec<RpcParam>,
}

#[derive(Debug, Clone)]
pub(super) struct DecodedValue {
    pub(super) type_name: String,
    pub(super) value_sql: String,
    pub(super) tvp_rows: Option<Vec<Vec<String>>>,
}
