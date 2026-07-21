use uuid::Uuid;

/// Stable identity of the single personal registry in a local Desktop database.
///
/// One-way legacy migration targets use the same identity so that replacing a
/// fresh local database with a reviewed migrated database immediately exposes
/// the imported registry to Desktop commands.
pub const LOCAL_LAB_ID: Uuid = Uuid::from_u128(0x4d55_5249_4152_4300_0000_0000_0000_0001);
pub const LOCAL_USER_ID: Uuid = Uuid::from_u128(0x4d55_5249_4152_4300_0000_0000_0000_0002);
pub const LOCAL_OPERATOR_NAME: &str = "本地操作员";
