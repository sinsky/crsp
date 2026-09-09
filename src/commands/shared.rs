use crate::error::CrspError;

pub fn parse_version(value: Option<&str>) -> Result<Option<i32>, CrspError> {
    value
        .map(|value| {
            value
                .parse::<i32>()
                .map_err(|_| CrspError::Validation(format!("'{value}' is not a valid integer.")))
        })
        .transpose()
}
