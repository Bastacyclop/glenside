use super::language::{ComputeType, Language, PadType};
use egg::RecExpr;
use itertools::Itertools;
use ndarray::{s, ArrayD, Dimension, IxDyn};
use std::collections::hash_map::HashMap;

pub enum Value<DataType> {
    Tensor(ArrayD<DataType>),
    Access(Access<DataType>),
    Usize(usize),
    Shape(IxDyn),
    ComputeType(ComputeType),
    PadType(PadType),
}

pub struct Access<DataType> {
    pub tensor: ArrayD<DataType>,
    pub access_axis: usize,
}

pub type Environment<'a, DataType> = HashMap<&'a str, ArrayD<DataType>>;

pub fn interpret<DataType>(
    expr: &RecExpr<Language>,
    index: usize,
    env: &Environment<DataType>,
) -> Value<DataType>
where
    DataType: Copy
        + std::ops::Mul<Output = DataType>
        + num_traits::identities::One
        + num_traits::identities::Zero
        + std::cmp::PartialOrd
        + num_traits::Bounded,
{
    match &expr.as_ref()[index] {
        &Language::AccessSqueeze([access_id, axis_id]) => {
            let mut access = match interpret(expr, access_id as usize, env) {
                Value::Access(a) => a,
                _ => panic!(),
            };
            let axis = match interpret(expr, axis_id as usize, env) {
                Value::Usize(u) => u,
                _ => panic!(),
            };

            assert_eq!(
                access.tensor.shape()[axis],
                1,
                "Cannot squeeze an axis which is not equal to 1"
            );

            access.tensor = access.tensor.index_axis_move(ndarray::Axis(axis), 0);
            if axis < access.access_axis {
                access.access_axis -= 1;
            }

            Value::Access(access)
        }
        Language::PadType(t) => Value::PadType(*t),
        &Language::AccessPad([access_id, pad_type_id, axis_id, pad_before_id, pad_after_id]) => {
            let access = match interpret(expr, access_id as usize, env) {
                Value::Access(a) => a,
                _ => panic!(),
            };
            let pad_type = match interpret(expr, pad_type_id as usize, env) {
                Value::PadType(t) => t,
                _ => panic!(),
            };
            let axis = match interpret(expr, axis_id as usize, env) {
                Value::Usize(u) => u,
                _ => panic!(),
            };
            let pad_before = match interpret(expr, pad_before_id as usize, env) {
                Value::Usize(u) => u,
                _ => panic!(),
            };
            let pad_after = match interpret(expr, pad_after_id as usize, env) {
                Value::Usize(u) => u,
                _ => panic!(),
            };

            match pad_type {
                PadType::ZeroPadding => {
                    let mut before_shape = access.tensor.shape().to_vec();
                    before_shape[axis] = pad_before;
                    let mut after_shape = access.tensor.shape().to_vec();
                    after_shape[axis] = pad_after;

                    Value::Access(Access {
                        tensor: ndarray::stack(
                            ndarray::Axis(axis),
                            &[
                                // TODO(@gussmith) What's going on here...
                                ndarray::ArrayD::zeros(before_shape).to_owned().view(),
                                access.tensor.clone().view(),
                                ndarray::ArrayD::zeros(after_shape).to_owned().view(),
                            ],
                        )
                        .unwrap(),
                        access_axis: access.access_axis,
                    })
                }
            }
        }
        Language::ComputeType(t) => Value::ComputeType(t.clone()),
        &Language::Compute([compute_type_id, access_id]) => {
            let compute_type = match interpret(expr, compute_type_id as usize, env) {
                Value::ComputeType(t) => t,
                _ => panic!(),
            };
            let access = match interpret(expr, access_id as usize, env) {
                Value::Access(a) => a,
                _ => panic!(),
            };

            match compute_type {
                ComputeType::ElementwiseMul => Value::Access(Access {
                    access_axis: access.access_axis,
                    tensor: access
                        .tensor
                        .axis_iter(ndarray::Axis(access.access_axis))
                        .fold(
                            ndarray::ArrayBase::ones(
                                access.tensor.shape()[..access.access_axis]
                                    .iter()
                                    .cloned()
                                    .chain(
                                        access.tensor.shape()[access.access_axis + 1..]
                                            .iter()
                                            .cloned(),
                                    )
                                    .collect::<Vec<_>>()
                                    .as_slice(),
                            ),
                            |acc, t| acc * t,
                        ),
                }),
                ComputeType::ElementwiseAdd => Value::Access(Access {
                    access_axis: access.access_axis,
                    tensor: access
                        .tensor
                        .axis_iter(ndarray::Axis(access.access_axis))
                        .fold(
                            ndarray::ArrayBase::zeros(
                                access.tensor.shape()[..access.access_axis]
                                    .iter()
                                    .cloned()
                                    .chain(
                                        access.tensor.shape()[access.access_axis + 1..]
                                            .iter()
                                            .cloned(),
                                    )
                                    .collect::<Vec<_>>()
                                    .as_slice(),
                            ),
                            |acc, t| acc + t,
                        ),
                }),
                ComputeType::DotProduct => {
                    let reshaped = access
                        .tensor
                        .clone()
                        .into_shape(
                            std::iter::once(
                                access.tensor.shape()[..access.access_axis]
                                    .iter()
                                    .cloned()
                                    .product(),
                            )
                            .chain(access.tensor.shape()[access.access_axis..].iter().cloned())
                            .collect::<Vec<_>>(),
                        )
                        .unwrap();

                    let num_elements_per_vec: usize = access.tensor.shape()
                        [access.access_axis + 1..]
                        .iter()
                        .product();

                    let result = ndarray::arr1(
                        reshaped
                            .axis_iter(ndarray::Axis(0))
                            .map(|t| {
                                t.axis_iter(ndarray::Axis(0))
                                    .fold(
                                        ndarray::ArrayBase::ones([num_elements_per_vec]),
                                        |acc, vec| {
                                            let reshaped = vec
                                                .clone()
                                                .into_shape([num_elements_per_vec])
                                                .unwrap();

                                            ndarray::arr1(
                                                reshaped
                                                    .axis_iter(ndarray::Axis(0))
                                                    .zip(acc.axis_iter(ndarray::Axis(0)))
                                                    .map(|(a, b)| {
                                                        *a.into_scalar() * *b.into_scalar()
                                                    })
                                                    .collect::<Vec<_>>()
                                                    .as_slice(),
                                            )
                                        },
                                    )
                                    .sum()
                            })
                            .collect::<Vec<_>>()
                            .as_slice(),
                    );

                    let reshaped = result
                        .into_shape(&access.tensor.shape()[..access.access_axis])
                        .unwrap();

                    Value::Access(Access {
                        access_axis: reshaped.ndim(),
                        tensor: reshaped,
                    })
                }
                ComputeType::ReLU => Value::Access(Access {
                    tensor: access.tensor.mapv(|v| {
                        if v >= DataType::zero() {
                            v
                        } else {
                            DataType::zero()
                        }
                    }),
                    access_axis: access.access_axis,
                }),
                ComputeType::ReduceSum => Value::Access(Access {
                    tensor: access
                        .tensor
                        .clone()
                        .into_shape(
                            access.tensor.shape()[..access.access_axis]
                                .iter()
                                .cloned()
                                .chain(std::iter::once(
                                    access.tensor.shape()[access.access_axis..]
                                        .iter()
                                        .cloned()
                                        .product(),
                                ))
                                .collect::<Vec<_>>()
                                .as_slice(),
                        )
                        .unwrap()
                        .sum_axis(ndarray::Axis(access.access_axis)),
                    access_axis: access.access_axis,
                }),
                ComputeType::ReduceMax => Value::Access(Access {
                    tensor: access
                        .tensor
                        .clone()
                        .into_shape(
                            access.tensor.shape()[..access.access_axis]
                                .iter()
                                .cloned()
                                .chain(std::iter::once(
                                    access.tensor.shape()[access.access_axis..]
                                        .iter()
                                        .cloned()
                                        .product(),
                                ))
                                .collect::<Vec<_>>()
                                .as_slice(),
                        )
                        .unwrap()
                        .map_axis(ndarray::Axis(access.access_axis), |t| {
                            t.iter().fold(
                                DataType::min_value(),
                                |acc, v| if *v > acc { *v } else { acc },
                            )
                        }),
                    access_axis: access.access_axis,
                }),
            }
        }
        &Language::AccessCartesianProduct([a0_id, a1_id]) => {
            let (a0, a1) = match (
                interpret(expr, a0_id as usize, env),
                interpret(expr, a1_id as usize, env),
            ) {
                (Value::Access(a0), Value::Access(a1)) => (a0, a1),
                _ => panic!(),
            };

            assert_eq!(
                a0.tensor.shape()[a0.access_axis..],
                a1.tensor.shape()[a1.access_axis..]
            );

            let reshaped_0 = a0
                .tensor
                .clone()
                .into_shape(
                    std::iter::once(
                        a0.tensor.shape()[..a0.access_axis]
                            .iter()
                            .cloned()
                            .product(),
                    )
                    .chain(a0.tensor.shape()[a0.access_axis..].iter().cloned())
                    .collect::<Vec<_>>(),
                )
                .unwrap();
            let reshaped_1 = a1
                .tensor
                .clone()
                .into_shape(
                    std::iter::once(
                        a1.tensor.shape()[..a1.access_axis]
                            .iter()
                            .cloned()
                            .product(),
                    )
                    .chain(a1.tensor.shape()[a1.access_axis..].iter().cloned())
                    .collect::<Vec<_>>(),
                )
                .unwrap();

            let to_stack = reshaped_0
                .axis_iter(ndarray::Axis(0))
                .cartesian_product(reshaped_1.axis_iter(ndarray::Axis(0)))
                .map(|(t0, t1)| {
                    ndarray::stack(
                        ndarray::Axis(0),
                        &[
                            t0.insert_axis(ndarray::Axis(0)),
                            t1.insert_axis(ndarray::Axis(0)),
                        ],
                    )
                    .unwrap()
                    .insert_axis(ndarray::Axis(0))
                })
                .collect::<Vec<_>>();

            let unreshaped = ndarray::stack(
                ndarray::Axis(0),
                to_stack
                    .iter()
                    .map(|t| t.view())
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
            .unwrap();

            let reshaped = unreshaped
                .into_shape(
                    a0.tensor.shape()[..a0.access_axis]
                        .iter()
                        .cloned()
                        .chain(a1.tensor.shape()[..a1.access_axis].iter().cloned())
                        .chain(std::iter::once(2))
                        .chain(a0.tensor.shape()[a0.access_axis..].iter().cloned())
                        .collect::<Vec<_>>(),
                )
                .unwrap();

            Value::Access(Access {
                tensor: reshaped.into_dyn(),
                access_axis: a0.access_axis + a1.access_axis,
            })
        }
        &Language::Access([access_id, dim_id]) => {
            let access = match interpret(expr, access_id as usize, env) {
                Value::Access(a) => a,
                _ => panic!(),
            };
            let dim = match interpret(expr, dim_id as usize, env) {
                Value::Usize(u) => u,
                _ => panic!(),
            };

            Value::Access(Access {
                tensor: access.tensor,
                // TODO(@gussmith) Settle on vocab: "axis" or "dimension"?
                access_axis: dim,
            })
        }
        &Language::AccessWindows([access_id, filters_shape_id, x_stride_id, y_stride_id]) => {
            let access = match interpret(expr, access_id as usize, env) {
                Value::Access(a) => a,
                _ => panic!(),
            };
            let filters_shape = match interpret(expr, filters_shape_id as usize, env) {
                Value::Shape(s) => s,
                _ => panic!(),
            };
            let x_stride = match interpret(expr, x_stride_id as usize, env) {
                Value::Usize(u) => u,
                _ => panic!(),
            };
            let y_stride = match interpret(expr, y_stride_id as usize, env) {
                Value::Usize(u) => u,
                _ => panic!(),
            };

            // Won't always have to be true. Just simplifying right now.
            assert_eq!(access.tensor.ndim(), 3);
            assert_eq!(access.access_axis, 3);
            assert_eq!(filters_shape.ndim(), 3);

            assert_eq!(access.tensor.ndim(), filters_shape.ndim());

            // TODO(@gussmith) Need one central place for window-gen logic
            // I'm duplicating this logic between here and language.rs. It
            // should be centralized.
            let (tensor_c, tensor_x, tensor_y) = (
                access.tensor.shape()[0],
                access.tensor.shape()[1],
                access.tensor.shape()[2],
            );
            let (filters_c, filters_x, filters_y) =
                (filters_shape[0], filters_shape[1], filters_shape[2]);
            // TODO(@gussmith) Channel stride is hardcoded to 1
            let num_windows_c = ((tensor_c - (filters_c - 1)) + 1 - 1) / 1;
            let num_windows_x = ((tensor_x - (filters_x - 1)) + x_stride - 1) / x_stride;
            let num_windows_y = ((tensor_y - (filters_y - 1)) + y_stride - 1) / y_stride;

            let windows = (0..num_windows_c)
                .map(|c_window_index: usize| {
                    let window_start_c = c_window_index * 1;
                    let windows = (0..num_windows_x)
                        .map(|x_window_index: usize| {
                            let window_start_x = x_window_index * x_stride;
                            let windows = (0..num_windows_y)
                                .map(|y_window_index: usize| {
                                    let window_start_y = y_window_index * y_stride;

                                    access
                                        .tensor
                                        .slice(s![
                                            window_start_c..window_start_c + filters_c,
                                            window_start_x..window_start_x + filters_x,
                                            window_start_y..window_start_y + filters_y
                                        ])
                                        .insert_axis(ndarray::Axis(0))
                                })
                                .collect::<Vec<_>>();
                            ndarray::stack(
                                ndarray::Axis(0),
                                windows
                                    .iter()
                                    .map(|t| t.view())
                                    .collect::<Vec<_>>()
                                    .as_slice(),
                            )
                            .unwrap()
                            .insert_axis(ndarray::Axis(0))
                        })
                        .collect::<Vec<_>>();
                    ndarray::stack(
                        ndarray::Axis(0),
                        windows
                            .iter()
                            .map(|t| t.view())
                            .collect::<Vec<_>>()
                            .as_slice(),
                    )
                    .unwrap()
                    .insert_axis(ndarray::Axis(0))
                })
                .collect::<Vec<_>>();
            let out = ndarray::stack(
                ndarray::Axis(0),
                windows
                    .iter()
                    .map(|t| t.view())
                    .collect::<Vec<_>>()
                    .as_slice(),
            )
            .unwrap();

            Value::Access(Access {
                tensor: out.into_dyn(),
                // TODO(@gussmith23) Hardcoded
                // This already bit me. I forgot to update it when I changed the
                // access-windows semantics, and it took me a bit to find the
                // bug.
                access_axis: 3,
            })
        }
        Language::Shape(list) => Value::Shape(IxDyn(
            list.iter()
                .map(|id: &u32| match interpret(expr, *id as usize, env) {
                    Value::Usize(u) => u,
                    _ => panic!(),
                })
                .collect::<Vec<_>>()
                .as_slice(),
        )),
        &Language::SliceShape([shape_id, slice_axis_id]) => match (
            interpret(expr, shape_id as usize, env),
            interpret(expr, slice_axis_id as usize, env),
        ) {
            (Value::Shape(s), Value::Usize(u)) => {
                Value::Shape(IxDyn(s.as_array_view().slice(s![u..]).to_slice().unwrap()))
            }
            _ => panic!(),
        },
        &Language::ShapeOf([tensor_id]) => match interpret(expr, tensor_id as usize, env) {
            Value::Tensor(t) => Value::Shape(IxDyn(t.shape())),
            _ => panic!(),
        },
        &Language::AccessTensor(tensor_id) => match interpret(expr, tensor_id as usize, env) {
            Value::Tensor(t) => Value::Access(Access {
                tensor: t,
                // TODO(@gussmith) Arbitrarily picked default access axis
                access_axis: 0,
            }),
            _ => panic!(),
        },
        Language::Symbol(s) => Value::Tensor(env[s.as_str()].clone()),
        &Language::Usize(u) => Value::Usize(u),

        &Language::MoveAxis(_)
        | &Language::CartesianProduct(_)
        | &Language::MapDotProduct(_)
        | &Language::Slice(_)
        | &Language::Concatenate(_)
        | &Language::ElementwiseAdd(_)
        | &Language::BsgSystolicArray(_)
        | &Language::SystolicArray(_)
        | &Language::AccessMoveAxis(_)
        | &Language::GetAccessShape(_)
        | &Language::AccessReshape(_)
        | &Language::AccessFlatten(_)
        | &Language::AccessShape(_)
        | &Language::AccessSlice(_)
        | &Language::AccessConcatenate(_)
        | &Language::AccessShiftRight(_)
        | &Language::AccessPair(_) => todo!(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndarray::array;
    use std::str::FromStr;

    #[test]
    fn compute_elementwise_add_0() {
        let mut env = Environment::new();
        env.insert(
            "t",
            array![[[1, -2], [3, 0]], [[-5, 6], [0, 8]], [[-9, 10], [11, 12]],].into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(compute elementwise-add
              (access (access-tensor t) 0)
             )",
        )
        .unwrap();

        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(access_axis, 0);
                assert_eq!(
                    tensor,
                    array![[1 + -5 + -9, -2 + 6 + 10], [3 + 0 + 11, 0 + 8 + 12]].into_dyn()
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn compute_elementwise_mul_0() {
        let mut env = Environment::new();
        env.insert(
            "t",
            array![[[1, -2], [3, 0]], [[-5, 6], [0, 8]], [[-9, 10], [11, 12]],].into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(compute elementwise-mul
              (access (access-tensor t) 0)
             )",
        )
        .unwrap();

        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(access_axis, 0);
                assert_eq!(
                    tensor,
                    array![[1 * -5 * -9, -2 * 6 * 10], [3 * 0 * 11, 0 * 8 * 12]].into_dyn()
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn compute_reduce_sum_0() {
        let mut env = Environment::new();
        env.insert(
            "t",
            array![[[1, -2], [3, 0]], [[-5, 6], [0, 8]], [[-9, 10], [11, 12]],].into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(compute reduce-sum
              (access (access-tensor t) 0)
             )",
        )
        .unwrap();

        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(access_axis, 0);
                assert_eq!(
                    tensor,
                    ndarray::arr0(1 + -2 + 3 + 0 + -5 + 6 + 0 + 8 + -9 + 10 + 11 + 12).into_dyn()
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn compute_reduce_sum_1() {
        let mut env = Environment::new();
        env.insert(
            "t",
            array![[[1, -2], [3, 0]], [[-5, 6], [0, 8]], [[-9, 10], [11, 12]],].into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(compute reduce-sum
              (access (access-tensor t) 1)
             )",
        )
        .unwrap();

        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(access_axis, 1);
                assert_eq!(
                    tensor,
                    array![1 + -2 + 3 + 0, -5 + 6 + 0 + 8, -9 + 10 + 11 + 12].into_dyn()
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn compute_reduce_sum_2() {
        let mut env = Environment::new();
        env.insert(
            "t",
            array![[[1, -2], [3, 0]], [[-5, 6], [0, 8]], [[-9, 10], [11, 12]],].into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(compute reduce-sum
              (access (access-tensor t) 2)
             )",
        )
        .unwrap();

        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(access_axis, 2);
                assert_eq!(
                    tensor,
                    array![[1 + -2, 3 + 0], [-5 + 6, 0 + 8], [-9 + 10, 11 + 12]].into_dyn()
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn compute_reduce_sum_3() {
        let mut env = Environment::new();
        env.insert(
            "t",
            array![[[1, -2], [3, 0]], [[-5, 6], [0, 8]], [[-9, 10], [11, 12]],].into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(compute reduce-sum
              (access (access-tensor t) 3)
             )",
        )
        .unwrap();

        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(access_axis, 3);
                assert_eq!(
                    tensor,
                    array![[[1, -2], [3, 0]], [[-5, 6], [0, 8]], [[-9, 10], [11, 12]],].into_dyn(),
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn compute_relu_0() {
        let mut env = Environment::new();
        env.insert(
            "t",
            array![[[1, -2], [3, 0]], [[-5, 6], [0, 8]], [[-9, 10], [11, 12]],].into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(compute relu
              (access (access-tensor t) 0)
             )",
        )
        .unwrap();

        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(access_axis, 0);
                assert_eq!(
                    tensor,
                    array![[[1, 0], [3, 0]], [[0, 6], [0, 8]], [[0, 10], [11, 12]],].into_dyn(),
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn compute_relu_1() {
        let mut env = Environment::new();
        env.insert(
            "t",
            array![[[1, -2], [3, 0]], [[-5, 6], [0, 8]], [[-9, 10], [11, 12]],].into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(compute relu
              (access (access-tensor t) 2)
             )",
        )
        .unwrap();

        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(access_axis, 2);
                assert_eq!(
                    tensor,
                    array![[[1, 0], [3, 0]], [[0, 6], [0, 8]], [[0, 10], [11, 12]],].into_dyn(),
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn compute_dot_product_0() {
        let mut env = Environment::new();
        env.insert(
            "t",
            // 3 x 2 x 2
            array![[[1, 2], [3, 4]], [[5, 6], [7, 8]], [[9, 10], [11, 12]],].into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(compute dot-product
              (access (access-tensor t) 0)
             )",
        )
        .unwrap();

        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(tensor.shape(), &[] as &[usize]);
                assert_eq!(access_axis, 0);
                assert_eq!(
                    tensor,
                    ndarray::arr0(1 * 5 * 9 + 2 * 6 * 10 + 3 * 7 * 11 + 4 * 8 * 12).into_dyn()
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn compute_dot_product_1() {
        let mut env = Environment::new();
        env.insert(
            "t",
            // 3 x 2 x 2
            array![[[1, 2], [3, 4]], [[5, 6], [7, 8]], [[9, 10], [11, 12]],].into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(compute dot-product
              (access (access-tensor t) 1)
             )",
        )
        .unwrap();

        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(tensor.shape(), &[3]);
                assert_eq!(access_axis, 1);
                assert_eq!(
                    tensor,
                    array![11, 5 * 7 + 8 * 6, 9 * 11 + 10 * 12].into_dyn()
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn compute_dot_product_2() {
        let mut env = Environment::new();
        env.insert(
            "t",
            // 3 x 2 x 2
            array![[[1, 2], [3, 4]], [[5, 6], [7, 8]], [[9, 10], [11, 12]],].into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(compute dot-product
              (access (access-tensor t) 2)
             )",
        )
        .unwrap();

        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(tensor.shape(), &[3, 2]);
                assert_eq!(access_axis, 2);
                assert_eq!(
                    tensor,
                    array![[1 * 2, 3 * 4], [5 * 6, 7 * 8], [9 * 10, 11 * 12]].into_dyn()
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn access_cartesian_product() {
        let mut env = Environment::new();
        env.insert(
            "t0",
            // 3 x 2 x 2
            array![[[1, 2], [3, 4]], [[5, 6], [7, 8]], [[9, 10], [11, 12]],].into_dyn(),
        );
        env.insert(
            "t1",
            // 2 x 2 x 2
            array![[[13, 14], [15, 16]], [[17, 18], [19, 20]]].into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(access-cartesian-product
              (access (access-tensor t0) 2)
              (access (access-tensor t1) 2)
             )",
        )
        .unwrap();

        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(tensor.shape(), &[3, 2, 2, 2, 2, 2]);
                assert_eq!(access_axis, 4);
                assert_eq!(
                    tensor.slice(s![0, 0, 0, 0, .., ..]),
                    array![[1, 2], [13, 14]]
                );
                assert_eq!(
                    tensor.slice(s![2, 0, 1, 0, .., ..]),
                    array![[9, 10], [17, 18]]
                );
            }
            _ => panic!(),
        }
    }
    #[test]
    fn access() {
        let mut env = Environment::new();
        env.insert("t", array![[1., 2.], [3., 4.]].into_dyn());

        let expr = RecExpr::<Language>::from_str("(access (access-tensor t) 1)").unwrap();
        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(tensor, array![[1., 2.], [3., 4.]].into_dyn());
                assert_eq!(access_axis, 1);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn access_windows() {
        let mut env = Environment::new();
        env.insert(
            "t",
            array![
                [[1., 2., 3.], [4., 5., 6.], [7., 8., 9.]],
                [[10., 11., 12.], [13., 14., 15.], [16., 17., 18.]],
                [[19., 20., 21.], [22., 23., 24.], [25., 26., 27.]],
            ]
            .into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "
             (access-windows
              (access (access-tensor t) 3)
              (shape 3 2 2)
              1
              1
             )",
        )
        .unwrap();
        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(a) => {
                assert_eq!(a.access_axis, 3);
                assert_eq!(a.tensor.shape(), &[1, 2, 2, 3, 2, 2]);
                assert_eq!(
                    a.tensor.slice(s![0, 0, 0, .., .., ..]),
                    array![
                        [[1., 2.], [4., 5.]],
                        [[10., 11.], [13., 14.]],
                        [[19., 20.], [22., 23.]],
                    ]
                );
                assert_eq!(
                    a.tensor.slice(s![0, 1, 0, .., .., ..]),
                    array![
                        [[4., 5.], [7., 8.]],
                        [[13., 14.], [16., 17.]],
                        [[22., 23.], [25., 26.]],
                    ]
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn shape() {
        let expr = RecExpr::<Language>::from_str("(shape 1 2 3)").unwrap();
        match interpret(
            &expr,
            expr.as_ref().len() - 1,
            &Environment::<f32>::default(),
        ) {
            Value::Shape(s) => assert_eq!(s, IxDyn(&[1, 2, 3])),
            _ => panic!(),
        }
    }

    #[test]
    fn slice_shape_0() {
        let mut env = Environment::new();
        env.insert("t", array![[1., 2.], [3., 4.]].into_dyn());

        let expr = RecExpr::<Language>::from_str("(slice-shape (shape-of t) 0)").unwrap();
        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Shape(s) => assert_eq!(s, IxDyn(&[2, 2])),
            _ => panic!(),
        }
    }

    #[test]
    fn slice_shape_1() {
        let mut env = Environment::new();
        env.insert("t", array![[1., 2.], [3., 4.]].into_dyn());

        let expr = RecExpr::<Language>::from_str("(slice-shape (shape-of t) 1)").unwrap();
        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Shape(s) => assert_eq!(s, IxDyn(&[2])),
            _ => panic!(),
        }
    }

    #[test]
    fn slice_shape_2() {
        let mut env = Environment::new();
        env.insert("t", array![[1., 2.], [3., 4.]].into_dyn());

        let expr = RecExpr::<Language>::from_str("(slice-shape (shape-of t) 2)").unwrap();
        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Shape(s) => assert_eq!(s, IxDyn(&[])),
            _ => panic!(),
        }
    }

    #[test]
    fn shape_of() {
        let mut env = Environment::new();
        env.insert("t", array![[1., 2.], [3., 4.]].into_dyn());

        let expr = RecExpr::<Language>::from_str("(shape-of t)").unwrap();
        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Shape(s) => assert_eq!(s, IxDyn(&[2, 2])),
            _ => panic!(),
        }
    }

    #[test]
    fn usize() {
        let expr = RecExpr::<Language>::from_str("23").unwrap();
        match interpret(
            &expr,
            expr.as_ref().len() - 1,
            &Environment::<f32>::default(),
        ) {
            Value::Usize(23) => (),
            _ => panic!(),
        }
    }

    #[test]
    fn symbol() {
        let mut env = Environment::new();
        env.insert("t", array![[1., 2.], [3., 4.]].into_dyn());

        let expr = RecExpr::<Language>::from_str("t").unwrap();
        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Tensor(t) => assert_eq!(t, array![[1., 2.], [3., 4.]].into_dyn()),
            _ => panic!(),
        }
    }

    #[test]
    fn access_tensor() {
        let mut env = Environment::new();
        env.insert("t", array![[1., 2.], [3., 4.]].into_dyn());

        let expr = RecExpr::<Language>::from_str("(access-tensor t)").unwrap();
        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(tensor, array![[1., 2.], [3., 4.]].into_dyn());
                assert_eq!(access_axis, 0);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn pad_type() {
        let expr = RecExpr::<Language>::from_str("zero-padding").unwrap();
        match interpret::<i32>(&expr, expr.as_ref().len() - 1, &Environment::default()) {
            Value::PadType(PadType::ZeroPadding) => (),
            _ => panic!(),
        };
    }

    #[test]
    fn access_pad() {
        let mut env = Environment::new();
        env.insert("t", array![[1., 2.], [3., 4.]].into_dyn());

        let expr =
            RecExpr::<Language>::from_str("(access-pad (access-tensor t) zero-padding 0 2 4)")
                .unwrap();
        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(
                    tensor,
                    array![
                        [0., 0.],
                        [0., 0.],
                        [1., 2.],
                        [3., 4.],
                        [0., 0.],
                        [0., 0.],
                        [0., 0.],
                        [0., 0.]
                    ]
                    .into_dyn()
                );
                assert_eq!(access_axis, 0);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn compute_reduce_max_0() {
        let mut env = Environment::new();
        env.insert(
            "t",
            array![[[1, -2], [3, 0]], [[-5, 6], [0, 8]], [[-9, 10], [11, 12]],].into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(compute reduce-max
              (access (access-tensor t) 0)
             )",
        )
        .unwrap();

        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(access_axis, 0);
                assert_eq!(tensor, ndarray::arr0(12).into_dyn());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn compute_reduce_max_1() {
        let mut env = Environment::new();
        env.insert(
            "t",
            array![[[1, -2], [3, 0]], [[-5, 6], [0, 8]], [[-9, 10], [11, 12]],].into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(compute reduce-max
              (access (access-tensor t) 1)
             )",
        )
        .unwrap();

        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(access_axis, 1);
                assert_eq!(tensor, array![3, 8, 12].into_dyn());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn compute_reduce_max_2() {
        let mut env = Environment::new();
        env.insert(
            "t",
            array![[[1, -2], [3, 0]], [[-5, 6], [0, 8]], [[-9, 10], [11, 12]],].into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(compute reduce-max
              (access (access-tensor t) 2)
             )",
        )
        .unwrap();

        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(access_axis, 2);
                assert_eq!(tensor, array![[1, 3], [6, 8], [10, 12]].into_dyn());
            }
            _ => panic!(),
        }
    }

    #[test]
    fn compute_reduce_max_3() {
        let mut env = Environment::new();
        env.insert(
            "t",
            array![[[1, -2], [3, 0]], [[-5, 6], [0, 8]], [[-9, 10], [11, 12]],].into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(compute reduce-max
              (access (access-tensor t) 3)
             )",
        )
        .unwrap();

        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(access_axis, 3);
                assert_eq!(
                    tensor,
                    array![[[1, -2], [3, 0]], [[-5, 6], [0, 8]], [[-9, 10], [11, 12]],].into_dyn(),
                );
            }
            _ => panic!(),
        }
    }

    #[test]
    fn access_squeeze_0() {
        let mut env = Environment::new();
        env.insert("t", array![[1., 2.]].into_dyn());

        let expr = RecExpr::<Language>::from_str("(access-squeeze (access-tensor t) 0)").unwrap();
        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(tensor, array![1., 2.].into_dyn());
                assert_eq!(access_axis, 0);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn access_squeeze_1() {
        let mut env = Environment::new();
        env.insert("t", array![[1., 2.]].into_dyn());

        let expr = RecExpr::<Language>::from_str("(access-squeeze (access (access-tensor t) 1) 0)")
            .unwrap();
        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(tensor, array![1., 2.].into_dyn());
                assert_eq!(access_axis, 0);
            }
            _ => panic!(),
        }
    }

    #[test]
    #[should_panic]
    fn access_squeeze_panic() {
        let mut env = Environment::new();
        env.insert("t", array![[1., 2.]].into_dyn());

        let expr = RecExpr::<Language>::from_str("(access-squeeze (access (access-tensor t) 1) 1)")
            .unwrap();
        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(tensor, array![1., 2.].into_dyn());
                assert_eq!(access_axis, 0);
            }
            _ => panic!(),
        }
    }

    /// Example showing how access-windows can be used to implement max pooling
    /// (in addition to convolution)
    #[test]
    fn max_pool2d() {
        let mut env = Environment::new();
        env.insert(
            "t",
            array![
                [[1, -2, -4, 5], [3, 6, -8, 0]],
                [[-5, 6, -8, -10], [0, 0, 0, 8]],
                [[-9, -20, -15, 10], [-1, 2, 11, 12]],
            ]
            .into_dyn(),
        );

        let expr = RecExpr::<Language>::from_str(
            "(compute reduce-max
              (access-windows (access (access-tensor t) 3) (shape 1 2 2) 2 2)
             )",
        )
        .unwrap();
        match interpret(&expr, expr.as_ref().len() - 1, &env) {
            Value::Access(Access {
                tensor,
                access_axis,
            }) => {
                assert_eq!(access_axis, 3);
                assert_eq!(tensor.shape(), [3, 1, 2]);
                assert_eq!(tensor, array![[[6, 5]], [[6, 8]], [[2, 12]]].into_dyn(),);
            }
            _ => panic!(),
        }
    }
}
