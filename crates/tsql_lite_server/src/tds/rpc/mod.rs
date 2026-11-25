mod decode;
mod parser;
mod render;
pub(crate) mod temporal;
mod types;

pub use parser::{
    parse_rpc, CatalogProc, CursorOp, CursorRpcRequest, RpcProc, RpcRequest, SqlRpcRequest,
};
pub use render::{
    append_output_sentinel, build_param_preamble, build_param_preamble_with_decls,
    build_param_preamble_with_outputs, build_stored_proc_exec, build_stored_proc_exec_with_outputs,
};
pub use types::{OutputParamBinding, RpcParam, RpcRequest as RpcRequestType};

pub use parser::{
    parse_param_decl, CatalogRpcRequest, ExecuteRpcRequest, PrepExecRpcRequest, PrepareRpcRequest,
    StoredProcedureRpcRequest, UnprepareRpcRequest,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_param_preamble_scalar_and_tvp() {
        let params = vec![
            RpcParam {
                name: "@x".to_string(),
                type_name: "INT".to_string(),
                value_sql: "42".to_string(),
                tvp_rows: None,
                is_output: false,
            },
            RpcParam {
                name: "@tvp".to_string(),
                type_name: "dbo.IntList READONLY".to_string(),
                value_sql: "NULL".to_string(),
                tvp_rows: Some(vec![vec!["1".to_string()], vec!["2".to_string()]]),
                is_output: false,
            },
        ];
        let sql = build_param_preamble(&params);
        assert!(sql.contains("DECLARE @x INT = 42;"));
        assert!(sql.contains("DECLARE @tvp dbo.IntList;"));
        assert!(sql.contains("INSERT INTO @tvp VALUES (1);"));
        assert!(sql.contains("INSERT INTO @tvp VALUES (2);"));
    }

    #[test]
    fn normalize_decl_type_strips_readonly() {
        assert_eq!(render::normalize_decl_type("dbo.T READONLY"), "dbo.T");
        assert_eq!(render::normalize_decl_type("dbo.T"), "dbo.T");
    }

    #[test]
    fn build_stored_proc_exec_handles_named_duplicate_and_tvp_params() {
        let named = vec![
            RpcParam {
                name: "@a".to_string(),
                type_name: "INT".to_string(),
                value_sql: "10".to_string(),
                tvp_rows: None,
                is_output: false,
            },
            RpcParam {
                name: "@b".to_string(),
                type_name: "NVARCHAR(20)".to_string(),
                value_sql: "N'hello'".to_string(),
                tvp_rows: None,
                is_output: false,
            },
        ];
        assert_eq!(
            build_stored_proc_exec("dbo.usp_AddSample", &named),
            "EXEC dbo.usp_AddSample @a = 10, @b = N'hello'"
        );

        let dup = vec![
            RpcParam {
                name: "@optname".to_string(),
                type_name: "NVARCHAR(20)".to_string(),
                value_sql: "N'Platform'".to_string(),
                tvp_rows: None,
                is_output: false,
            },
            RpcParam {
                name: "@optname".to_string(),
                type_name: "NVARCHAR(20)".to_string(),
                value_sql: "N'ProductVersion'".to_string(),
                tvp_rows: None,
                is_output: false,
            },
        ];
        assert_eq!(
            build_stored_proc_exec("xp_msver", &dup),
            "EXEC xp_msver N'Platform', N'ProductVersion'"
        );

        let tvp = vec![RpcParam {
            name: "@Ids".to_string(),
            type_name: "dbo.IdList READONLY".to_string(),
            value_sql: "NULL".to_string(),
            tvp_rows: Some(vec![vec!["1".to_string()], vec!["2".to_string()]]),
            is_output: false,
        }];
        let sql = build_stored_proc_exec("dbo.usp_LoadByIds", &tvp);
        assert!(sql.contains("DECLARE @__rpc_tvp_0 dbo.IdList;"));
        assert!(sql.contains("INSERT INTO @__rpc_tvp_0 VALUES (1);"));
        assert!(sql.contains("EXEC dbo.usp_LoadByIds @Ids = @__rpc_tvp_0"));
    }

    #[test]
    fn parse_rpc_named_procedure_with_plp_and_guid_params() {
        // Build a minimal TDS 7.4 RPC frame:
        // ALL_HEADERS (4 bytes total_len=4) + proc name "dbo.usp_Lookup" + flags=0
        let mut frame: Vec<u8> = Vec::new();
        frame.extend_from_slice(&4u32.to_le_bytes()); // ALL_HEADERS total len
        let proc_name = "dbo.usp_Lookup";
        let utf16_name: Vec<u16> = proc_name.encode_utf16().collect();
        frame.extend_from_slice(&(utf16_name.len() as u16).to_le_bytes());
        for u in utf16_name {
            frame.extend_from_slice(&u.to_le_bytes());
        }
        frame.extend_from_slice(&0u16.to_le_bytes()); // OptionFlags

        // Param 1: @ItemGuid UNIQUEIDENTIFIER (type 0x24, max_len=16, actual_len=16, 16 bytes)
        let p1 = "@ItemGuid";
        let p1_u16: Vec<u16> = p1.encode_utf16().collect();
        frame.push(p1_u16.len() as u8);
        for u in p1_u16 {
            frame.extend_from_slice(&u.to_le_bytes());
        }
        frame.push(0x00); // status flags
        frame.push(0x24); // GUIDTYPE
        frame.push(0x10); // max_len = 16
        frame.push(0x10); // actual_len = 16
        frame.extend_from_slice(&[
            0x78, 0x56, 0x34, 0x12, 0xBC, 0x9A, 0xF0, 0xDE, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66,
            0x77, 0x88,
        ]);

        // Param 2: @Note NVARCHAR(MAX) PLP (type 0xE7, max_len=0xFFFF, collation=5 bytes, PLP)
        let p2 = "@Note";
        let p2_u16: Vec<u16> = p2.encode_utf16().collect();
        frame.push(p2_u16.len() as u8);
        for u in p2_u16 {
            frame.extend_from_slice(&u.to_le_bytes());
        }
        frame.push(0x00); // status flags
        frame.push(0xE7); // NVARCHARTYPE
        frame.extend_from_slice(&0xFFFFu16.to_le_bytes()); // MAX (PLP)
        frame.extend_from_slice(&[0x09, 0x04, 0xD0, 0x00, 0x34]); // collation
        let note_bytes: Vec<u8> = "hello"
            .encode_utf16()
            .flat_map(|u| u.to_le_bytes())
            .collect();
        frame.extend_from_slice(&(note_bytes.len() as u64).to_le_bytes()); // PLP total len
        frame.extend_from_slice(&(note_bytes.len() as u32).to_le_bytes()); // chunk len
        frame.extend_from_slice(&note_bytes);
        frame.extend_from_slice(&0u32.to_le_bytes()); // PLP terminator

        let parsed = parse_rpc(&frame).unwrap().expect("should parse named RPC");
        match parsed {
            RpcRequest::StoredProcedure(sp) => {
                assert_eq!(sp.name, "dbo.usp_Lookup");
                assert_eq!(sp.params.len(), 2);
                assert_eq!(sp.params[0].name, "@ItemGuid");
                assert_eq!(
                    sp.params[0].value_sql,
                    "'12345678-9ABC-DEF0-1122-334455667788'"
                );
                assert_eq!(sp.params[1].name, "@Note");
                assert_eq!(sp.params[1].value_sql, "N'hello'");
            }
            other => panic!("expected StoredProcedure, got {other:?}"),
        }
    }
}
