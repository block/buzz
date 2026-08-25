use crate::b2::DomainError;
use rust_decimal::{prelude::ToPrimitive, Decimal, RoundingStrategy};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct AllocationTarget {
    pub sales_order_id: Uuid,
    pub weight: Decimal,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AllocationResult {
    pub sales_order_id: Uuid,
    pub weight: Decimal,
    pub amount: Decimal,
    pub remainder_rank: usize,
}

/// Allocate a two-decimal currency amount with largest-remainder rounding.
/// Ties are resolved by stable sales-order UUID bytes.
pub fn largest_remainder(
    amount: Decimal,
    targets: &[AllocationTarget],
) -> Result<Vec<AllocationResult>, DomainError> {
    if amount <= Decimal::ZERO || amount.round_dp(2) != amount {
        return Err(DomainError::Invalid(
            "adjustment amount must be positive with at most two decimals".into(),
        ));
    }
    if targets.is_empty() || targets.iter().any(|target| target.weight < Decimal::ZERO) {
        return Err(DomainError::Invalid(
            "allocation requires non-negative targets".into(),
        ));
    }
    let mut combined = BTreeMap::<Uuid, Decimal>::new();
    for target in targets {
        *combined.entry(target.sales_order_id).or_default() += target.weight;
    }
    let total: Decimal = combined.values().copied().sum();
    if total <= Decimal::ZERO {
        return Err(DomainError::Invalid(
            "allocation target weight must be greater than zero".into(),
        ));
    }
    let mut rows: Vec<(Uuid, Decimal, Decimal, Decimal)> = combined
        .into_iter()
        .map(|(id, weight)| {
            let exact = amount * weight / total;
            let floor = exact.round_dp_with_strategy(2, RoundingStrategy::ToZero);
            (id, weight, floor, exact - floor)
        })
        .collect();
    rows.sort_by(|left, right| {
        right
            .3
            .cmp(&left.3)
            .then_with(|| left.0.as_bytes().cmp(right.0.as_bytes()))
    });
    let floor_total: Decimal = rows.iter().map(|row| row.2).sum();
    let cents = ((amount - floor_total) * Decimal::new(100, 0))
        .to_u64()
        .ok_or_else(|| DomainError::Invalid("allocation remainder is invalid".into()))?;
    for index in 0..usize::try_from(cents).unwrap_or(usize::MAX) {
        let row = rows
            .get_mut(index)
            .ok_or_else(|| DomainError::Invalid("allocation remainder exceeds targets".into()))?;
        row.2 += Decimal::new(1, 2);
    }
    Ok(rows
        .into_iter()
        .enumerate()
        .map(
            |(rank, (sales_order_id, weight, amount, _))| AllocationResult {
                sales_order_id,
                weight,
                amount,
                remainder_rank: rank,
            },
        )
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn decimal(value: &str) -> Decimal {
        Decimal::from_str(value).unwrap()
    }

    #[test]
    fn deterministic_tail_goes_to_stable_id() {
        let low = Uuid::from_u128(1);
        let high = Uuid::from_u128(2);
        let rows = largest_remainder(
            decimal("10.01"),
            &[
                AllocationTarget {
                    sales_order_id: high,
                    weight: Decimal::ONE,
                },
                AllocationTarget {
                    sales_order_id: low,
                    weight: Decimal::ONE,
                },
            ],
        )
        .unwrap();
        assert_eq!(
            rows.iter().map(|row| row.amount).sum::<Decimal>(),
            decimal("10.01")
        );
        assert_eq!(rows[0].sales_order_id, low);
        assert_eq!(rows[0].amount, decimal("5.01"));
    }

    #[test]
    fn zero_weight_fails_closed() {
        assert!(largest_remainder(
            decimal("1"),
            &[AllocationTarget {
                sales_order_id: Uuid::new_v4(),
                weight: Decimal::ZERO,
            }]
        )
        .is_err());
    }
}
