use crate::errors::{InternalError, LVInteropError};

#[repr(transparent)]
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct LVArrayDims<const D: usize>([i32; D]);

impl<const D: usize> LVArrayDims<D> {
    pub fn new_empty() -> Self {
        Self([0; D])
    }

    pub fn shape(&self) -> [i32; D] {
        self.0
    }
    pub fn element_count(&self) -> usize {
        self.0
            .iter()
            .try_fold(1usize, |size, dim| {
                let dim_usize = usize::try_from(*dim).ok()?;
                size.checked_mul(dim_usize)
            })
            .unwrap_or(0)
    }
}

impl<const D: usize> From<[i32; D]> for LVArrayDims<D> {
    fn from(dim_sizes: [i32; D]) -> Self {
        Self(dim_sizes)
    }
}

impl<const D: usize> TryFrom<&[usize; D]> for LVArrayDims<D> {
    type Error = LVInteropError;

    fn try_from(value: &[usize; D]) -> Result<Self, Self::Error> {
        let mut dimensions = [0i32; D];

        for (into, &from) in dimensions.iter_mut().zip(value.iter()) {
            *into = from
                .try_into()
                .map_err(|_| LVInteropError::from(InternalError::ArrayDimensionsOutOfRange))?
        }
        Ok(dimensions.into())
    }
}

impl<const D: usize> TryFrom<&[usize]> for LVArrayDims<D> {
    type Error = LVInteropError;

    fn try_from(value: &[usize]) -> Result<Self, Self::Error> {
        let array: &[usize; D] = value
            .try_into()
            .map_err(|_| LVInteropError::from(InternalError::ArrayDimensionMismatch))?;
        array.try_into()
    }
}

impl<const D: usize> LVArrayDims<D> {
    /// Try to convert to a `[usize; D]` array.
    ///
    /// Returns an error if any dimension is negative.
    ///
    /// This is a non-panicking alternative to the `From<LVArrayDims<D>>` for `[usize; D]`
    /// conversion.
    pub fn try_to_usize_array(&self) -> Result<[usize; D], LVInteropError> {
        let mut usize_values = [0usize; D];
        for (output, input) in usize_values.iter_mut().zip(self.0.iter()) {
            *output = (*input)
                .try_into()
                .map_err(|_| LVInteropError::from(InternalError::ArrayDimensionsOutOfRange))?;
        }
        Ok(usize_values)
    }
}

impl<const D: usize> From<LVArrayDims<D>> for [usize; D] {
    /// Convert to the usize version.
    ///
    /// # Panics
    ///
    /// Panics if any dimension is negative. Use `TryFrom` for a
    /// non-panicking alternative.
    fn from(value: LVArrayDims<D>) -> Self {
        let mut usize_values = [0usize; D];
        for (output, input) in usize_values.iter_mut().zip(value.0.iter()) {
            *output = (*input).try_into().expect("Negative dimension size.");
        }
        usize_values
    }
}

/// Implement named methods for the first dimensions.
impl LVArrayDims<2> {
    pub fn rows(&self) -> i32 {
        self.0[0]
    }

    pub fn columns(&self) -> i32 {
        self.0[1]
    }
}

impl LVArrayDims<3> {
    pub fn rows(&self) -> i32 {
        self.0[1]
    }

    pub fn columns(&self) -> i32 {
        self.0[2]
    }

    pub fn pages(&self) -> i32 {
        self.0[0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_dims_empty() {
        let ref_dims = LVArrayDims::<3>([0, 0, 0]);
        let dims = LVArrayDims::<3>::new_empty();
        assert_eq!(dims, ref_dims);
        assert_eq!(dims.element_count(), 0);
    }

    #[test]
    fn dimension_element_count() {
        let dims = LVArrayDims::<3>([2, 3, 4]);
        assert_eq!(dims.element_count(), 24);

        let dims = LVArrayDims::<2>([2, 3]);
        assert_eq!(dims.element_count(), 6);

        let dims = LVArrayDims::<1>([2]);
        assert_eq!(dims.element_count(), 2);
    }

    #[test]
    fn test_dim_equality() {
        let dims1 = LVArrayDims::<3>([2, 3, 4]);
        let dims2 = LVArrayDims::<3>([2, 3, 4]);
        assert_eq!(dims1, dims2);
    }

    #[test]
    fn test_dims_from_usize_ok() {
        let dims = &[1usize, 2usize];
        let lvdims: LVArrayDims<2> = dims.try_into().unwrap();
        assert_eq!(lvdims, [1i32, 2].into())
    }

    #[test]
    fn test_dims_from_usize_out_of_range() {
        let dims = &[1usize, i32::MAX as usize + 1];
        let result: Result<LVArrayDims<2>, _> = dims.try_into();

        let _expected_err: Result<LVArrayDims<2>, _> = Err(LVInteropError::from(
            InternalError::ArrayDimensionsOutOfRange,
        ));
        assert!(matches!(result, _expected_err));
    }

    #[test]
    fn test_access_dims() {
        let dims = LVArrayDims::<2>([2, 3]);
        assert_eq!(dims.shape(), [2, 3]);
    }

    #[test]
    fn test_2d_dim_names() {
        let dims = LVArrayDims::<2>([2, 3]);
        assert_eq!(dims.rows(), 2);
        assert_eq!(dims.columns(), 3);
    }

    #[test]
    fn test_3d_dim_names() {
        let dims = LVArrayDims::<3>([2, 3, 4]);
        assert_eq!(dims.rows(), 3);
        assert_eq!(dims.columns(), 4);
        assert_eq!(dims.pages(), 2);
    }

    #[test]
    fn element_count_returns_zero_for_negative_dimension() {
        let dims = LVArrayDims::<2>([2, -1]);
        assert_eq!(dims.element_count(), 0);
    }

    #[test]
    fn element_count_handles_large_dimensions_without_panic() {
        // Verify that large valid dimensions don't cause panics via overflow
        let dims = LVArrayDims::<2>([i32::MAX, i32::MAX]);
        // The result should be a well-defined value (no UB from overflow)
        let _ = dims.element_count();
    }

    #[test]
    fn try_to_usize_array_ok() {
        let dims = LVArrayDims::<2>([2, 3]);
        let result = dims.try_to_usize_array();
        assert_eq!(result.unwrap(), [2, 3]);
    }

    #[test]
    fn try_to_usize_array_negative_dimension() {
        let dims = LVArrayDims::<2>([2, -1]);
        let result = dims.try_to_usize_array();
        assert!(result.is_err());
    }
}
