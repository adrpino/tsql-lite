use crate::ast::DataTypeSpec;
use crate::catalog::Catalog;
use crate::types::DataType;

pub(crate) fn data_type_spec_to_runtime(spec: &DataTypeSpec) -> DataType {
    match spec {
        DataTypeSpec::Bit => DataType::Bit,
        DataTypeSpec::TinyInt => DataType::TinyInt,
        DataTypeSpec::SmallInt => DataType::SmallInt,
        DataTypeSpec::Int => DataType::Int,
        DataTypeSpec::BigInt => DataType::BigInt,
        DataTypeSpec::Float => DataType::Float,
        DataTypeSpec::Decimal(p, s) => DataType::Decimal {
            precision: *p,
            scale: *s,
        },
        DataTypeSpec::Money => DataType::Money,
        DataTypeSpec::SmallMoney => DataType::SmallMoney,
        DataTypeSpec::Char(len) => DataType::Char { len: *len },
        DataTypeSpec::VarChar(max_len) => DataType::VarChar { max_len: *max_len },
        DataTypeSpec::NChar(len) => DataType::NChar { len: *len },
        DataTypeSpec::NVarChar(max_len) => DataType::NVarChar { max_len: *max_len },
        DataTypeSpec::Binary(len) => DataType::Binary { len: *len },
        DataTypeSpec::VarBinary(max_len) => DataType::VarBinary { max_len: *max_len },
        DataTypeSpec::Vector(dimensions) => DataType::Vector {
            dimensions: *dimensions,
        },
        DataTypeSpec::Date => DataType::Date,
        DataTypeSpec::Time => DataType::Time,
        DataTypeSpec::DateTime => DataType::DateTime,
        DataTypeSpec::DateTime2 => DataType::DateTime2,
        DataTypeSpec::SmallDateTime => DataType::SmallDateTime,
        DataTypeSpec::DateTimeOffset => DataType::DateTimeOffset,
        DataTypeSpec::UniqueIdentifier => DataType::UniqueIdentifier,
        DataTypeSpec::SqlVariant => DataType::SqlVariant,
        DataTypeSpec::Numeric(p, s) => DataType::Decimal {
            precision: *p,
            scale: *s,
        },
        DataTypeSpec::Xml => DataType::Xml,
        DataTypeSpec::RowVersion => DataType::Binary { len: 8 },
        DataTypeSpec::Custom(_) => DataType::VarChar { max_len: 255 },
    }
}

pub(crate) fn resolve_data_type_spec(spec: &DataTypeSpec, catalog: &dyn Catalog) -> DataType {
    resolve_data_type_spec_with_nullability(spec, catalog).0
}

pub(crate) fn resolve_data_type_spec_with_nullability(
    spec: &DataTypeSpec,
    catalog: &dyn Catalog,
) -> (DataType, Option<bool>) {
    if let DataTypeSpec::Custom(obj) = spec {
        let schema = obj.schema_or_dbo();
        if let Some(def) = catalog.find_scalar_type(schema, &obj.name) {
            return (def.base_type.clone(), Some(def.nullable));
        }
        // Fallback: if schema wasn't specified, also check dbo explicitly (already schema_or_dbo)
        // or single-part name match across schemas if only one exists.
        if obj.schema.is_none() {
            if let Some(def) = catalog
                .get_scalar_types()
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(&obj.name))
            {
                return (def.base_type.clone(), Some(def.nullable));
            }
        }
    }
    (data_type_spec_to_runtime(spec), None)
}
