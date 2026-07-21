use std::io::Write;

use rust_xlsxwriter::Workbook;
use serde::{Deserialize, Serialize};

use crate::{ImportError, safe_csv_cell};

pub const ANIMAL_IMPORT_HEADERS: [&str; 8] = [
    "display_id",
    "sex",
    "birth_date",
    "strain",
    "cage",
    "genotype",
    "father",
    "mother",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnimalImportFieldType {
    String,
    Enum,
    Date,
    Reference,
    CanonicalGenotype,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalImportFieldSpec {
    pub key: String,
    pub label: String,
    pub data_type: AnimalImportFieldType,
    pub required: bool,
    pub legal_values: Vec<String>,
    pub description: String,
    pub example: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalImportExample {
    pub display_id: String,
    pub sex: String,
    pub birth_date: String,
    pub strain: String,
    pub cage: String,
    pub genotype: String,
    pub father: String,
    pub mother: String,
}

impl AnimalImportExample {
    fn values(&self) -> [&str; ANIMAL_IMPORT_HEADERS.len()] {
        [
            &self.display_id,
            &self.sex,
            &self.birth_date,
            &self.strain,
            &self.cage,
            &self.genotype,
            &self.father,
            &self.mother,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnimalImportSchema {
    pub version: u32,
    pub fields: Vec<AnimalImportFieldSpec>,
    pub genotype_syntax: String,
    pub examples: Vec<AnimalImportExample>,
}

pub fn animal_import_schema() -> AnimalImportSchema {
    let field = |key: &str,
                 label: &str,
                 data_type,
                 required,
                 legal_values: &[&str],
                 description: &str,
                 example: &str| AnimalImportFieldSpec {
        key: key.to_owned(),
        label: label.to_owned(),
        data_type,
        required,
        legal_values: legal_values
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        description: description.to_owned(),
        example: example.to_owned(),
    };
    AnimalImportSchema {
        version: 1,
        fields: vec![
            field(
                "display_id",
                "动物显示编号",
                AnimalImportFieldType::String,
                true,
                &[],
                "在当前编号 scope 内唯一；不能为空。",
                "M-26001",
            ),
            field(
                "sex",
                "性别",
                AnimalImportFieldType::Enum,
                false,
                &["male", "female", "unknown"],
                "支持生产解析器认可的中英文别名；模板推荐使用标准英文值。",
                "male",
            ),
            field(
                "birth_date",
                "出生日期",
                AnimalImportFieldType::Date,
                false,
                &["YYYY-MM-DD"],
                "推荐 ISO 日期格式。",
                "2026-07-01",
            ),
            field(
                "strain",
                "品系",
                AnimalImportFieldType::String,
                false,
                &[],
                "动物品系名称。",
                "C57BL/6J",
            ),
            field(
                "cage",
                "笼位",
                AnimalImportFieldType::Reference,
                false,
                &["display_id", "section/display_id"],
                "编号唯一时可只写编号；有歧义时必须写 section/display_id。笼位必须已存在。",
                "A/A03",
            ),
            field(
                "genotype",
                "基因型",
                AnimalImportFieldType::CanonicalGenotype,
                false,
                &["{Locus}[allele_1]/[allele_2]&{AnotherLocus}[allele_1]/[allele_2]"],
                "位点、allele 及完全匹配的未归档 Genetics v2 定义必须已存在；不会静默创建或转换。",
                "{Trp53}[+]/[flox]&{Cre}[Cre]/[+]",
            ),
            field(
                "father",
                "父本",
                AnimalImportFieldType::Reference,
                false,
                &[],
                "父本显示编号；可引用已存在动物或本文件中更早/同批定义的动物。",
                "M-25010",
            ),
            field(
                "mother",
                "母本",
                AnimalImportFieldType::Reference,
                false,
                &[],
                "母本显示编号；可引用已存在动物或本文件中更早/同批定义的动物。",
                "F-25011",
            ),
        ],
        genotype_syntax: "{Locus}[allele_1]/[allele_2]&{AnotherLocus}[allele_1]/[allele_2]"
            .to_owned(),
        examples: vec![
            AnimalImportExample {
                display_id: "M-26001".to_owned(),
                sex: "male".to_owned(),
                birth_date: "2026-07-01".to_owned(),
                strain: "C57BL/6J".to_owned(),
                cage: "A/A03".to_owned(),
                genotype: "{Trp53}[+]/[flox]&{Cre}[Cre]/[+]".to_owned(),
                father: "M-25010".to_owned(),
                mother: "F-25011".to_owned(),
            },
            AnimalImportExample {
                display_id: "M-26002".to_owned(),
                sex: "female".to_owned(),
                birth_date: "2026-07-02".to_owned(),
                strain: "BALB/c".to_owned(),
                cage: "A/A03".to_owned(),
                genotype: "{Rosa26}[tdT]/[+]".to_owned(),
                father: "M-25010".to_owned(),
                mother: "F-25011".to_owned(),
            },
        ],
    }
}

pub fn animal_import_template_csv(writer: impl Write) -> Result<(), ImportError> {
    let schema = animal_import_schema();
    let mut csv = csv::Writer::from_writer(writer);
    csv.write_record(ANIMAL_IMPORT_HEADERS)?;
    for example in &schema.examples {
        csv.write_record(example.values().into_iter().map(safe_csv_cell))?;
    }
    csv.flush()?;
    Ok(())
}

pub fn animal_import_template_xlsx() -> Result<Vec<u8>, ImportError> {
    let schema = animal_import_schema();
    let mut workbook = Workbook::new();
    {
        let worksheet = workbook.add_worksheet();
        worksheet.set_name("animals")?;
        for (column, header) in ANIMAL_IMPORT_HEADERS.iter().enumerate() {
            worksheet.write_string(0, column as u16, *header)?;
        }
        for (row, example) in schema.examples.iter().enumerate() {
            for (column, value) in example.values().iter().enumerate() {
                worksheet.write_string((row + 1) as u32, column as u16, *value)?;
            }
        }
    }
    {
        let worksheet = workbook.add_worksheet();
        worksheet.set_name("instructions")?;
        for (column, header) in [
            "field",
            "label",
            "type",
            "required",
            "legal_values",
            "description",
            "example",
        ]
        .iter()
        .enumerate()
        {
            worksheet.write_string(0, column as u16, *header)?;
        }
        for (row, field) in schema.fields.iter().enumerate() {
            let row = (row + 1) as u32;
            worksheet.write_string(row, 0, &field.key)?;
            worksheet.write_string(row, 1, &field.label)?;
            worksheet.write_string(row, 2, format!("{:?}", field.data_type).to_lowercase())?;
            worksheet.write_string(row, 3, if field.required { "yes" } else { "no" })?;
            worksheet.write_string(row, 4, field.legal_values.join(" | "))?;
            worksheet.write_string(row, 5, &field.description)?;
            worksheet.write_string(row, 6, &field.example)?;
        }
    }
    Ok(workbook.save_to_buffer()?)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::*;
    use crate::{FieldMapping, preview_animals, read_csv, read_xlsx};

    #[test]
    fn generated_templates_follow_the_same_schema_and_parse() {
        let schema = animal_import_schema();
        assert_eq!(
            schema
                .fields
                .iter()
                .map(|field| field.key.as_str())
                .collect::<Vec<_>>(),
            ANIMAL_IMPORT_HEADERS
        );

        let mut csv = Vec::new();
        animal_import_template_csv(&mut csv).unwrap();
        let table = read_csv(Cursor::new(csv)).unwrap();
        assert_eq!(table.headers, ANIMAL_IMPORT_HEADERS);
        let preview = preview_animals(&table, &FieldMapping::infer(&table.headers));
        assert_eq!(preview.total_rows, schema.examples.len());
        assert!(preview.can_confirm());

        let xlsx = animal_import_template_xlsx().unwrap();
        let table = read_xlsx(Cursor::new(xlsx)).unwrap();
        assert_eq!(table.headers, ANIMAL_IMPORT_HEADERS);
        assert_eq!(table.rows.len(), schema.examples.len());
    }
}
