// This file is part of Substrate.

// Copyright (C) Parity Technologies (UK) Ltd.
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![cfg(feature = "runtime-benchmarks")]

use super::*;

use crate::{CoreAssignment::Task, Pallet as Broker};
use alloc::{vec, vec::Vec};
use frame_benchmarking::v2::*;
use frame_support::{
	storage::bounded_vec::BoundedVec,
	traits::{
		fungible::{Inspect, Mutate},
		EnsureOrigin, Hooks,
	},
};
use frame_system::{Pallet as System, RawOrigin};
use sp_arithmetic::{traits::Zero, Perbill};
use sp_core::Get;
use sp_runtime::{
	traits::{BlockNumberProvider, MaybeConvert},
	SaturatedConversion, Saturating,
};

const SEED: u32 = 0;
const MAX_CORE_COUNT: u16 = 1_000;

fn assert_last_event<T: Config>(generic_event: <T as Config>::RuntimeEvent) {
	frame_system::Pallet::<T>::assert_last_event(generic_event.into());
}

fn assert_has_event<T: Config>(generic_event: <T as Config>::RuntimeEvent) {
	frame_system::Pallet::<T>::assert_has_event(generic_event.into());
}

fn new_config_record<T: Config>() -> ConfigRecordOf<T> {
	ConfigRecord {
		advance_notice: 2u32.into(),
		interlude_length: 1u32.into(),
		leadin_length: 1u32.into(),
		ideal_bulk_proportion: Default::default(),
		limit_cores_offered: None,
		region_length: 3,
		renewal_bump: Perbill::from_percent(10),
		contribution_timeout: 5,
	}
}

fn new_schedule() -> Schedule {
	// Max items for worst case
	let mut items = Vec::new();
	for i in 0..CORE_MASK_BITS {
		items.push(ScheduleItem {
			assignment: Task(i.try_into().unwrap()),
			mask: CoreMask::complete(),
		});
	}
	Schedule::truncate_from(items)
}

fn setup_reservations<T: Config>(n: u32) {
	let schedule = new_schedule();

	Reservations::<T>::put(BoundedVec::try_from(vec![schedule.clone(); n as usize]).unwrap());
}

fn setup_leases<T: Config>(n: u32, task: u32, until: u32) {
	Leases::<T>::put(
		BoundedVec::try_from(vec![LeaseRecordItem { task, until: until.into() }; n as usize])
			.unwrap(),
	);
}

fn advance_to<T: Config>(b: u32) {
	while System::<T>::block_number() < b.into() {
		System::<T>::set_block_number(System::<T>::block_number().saturating_add(1u32.into()));

		let block_number: u32 = System::<T>::block_number().try_into().ok().unwrap();

		RCBlockNumberProviderOf::<T::Coretime>::set_block_number(block_number.into());
		Broker::<T>::on_initialize(System::<T>::block_number());
	}
}

fn setup_and_start_sale<T: Config>() -> Result<u16, BenchmarkError> {
	Configuration::<T>::put(new_config_record::<T>());

	// Assume Reservations to be filled for worst case
	setup_reservations::<T>(T::MaxReservedCores::get());

	// Assume Leases to be filled for worst case
	setup_leases::<T>(T::MaxLeasedCores::get(), 1, 10);

	Broker::<T>::do_start_sales(10_000_000u32.into(), MAX_CORE_COUNT.into())
		.map_err(|_| BenchmarkError::Weightless)?;

	Ok(T::MaxReservedCores::get()
		.saturating_add(T::MaxLeasedCores::get())
		.try_into()
		.unwrap())
}

#[benchmarks]
mod benches {
	use super::*;
	use crate::Finality::*;

	#[benchmark]
	fn configure() -> Result<(), BenchmarkError> {
		let config = new_config_record::<T>();

		let origin =
			T::AdminOrigin::try_successful_origin().map_err(|_| BenchmarkError::Weightless)?;

		#[extrinsic_call]
		_(origin as T::RuntimeOrigin, config.clone());

		assert_eq!(Configuration::<T>::get(), Some(config));

		Ok(())
	}

	#[benchmark]
	fn reserve() -> Result<(), BenchmarkError> {
		let schedule = new_schedule();

		// Assume Reservations to be almost filled for worst case
		setup_reservations::<T>(T::MaxReservedCores::get().saturating_sub(1));

		let origin =
			T::AdminOrigin::try_successful_origin().map_err(|_| BenchmarkError::Weightless)?;

		#[extrinsic_call]
		_(origin as T::RuntimeOrigin, schedule);

		assert_eq!(Reservations::<T>::get().len(), T::MaxReservedCores::get() as usize);

		Ok(())
	}

	#[benchmark]
	fn unreserve() -> Result<(), BenchmarkError> {
		// Assume Reservations to be filled for worst case
		setup_reservations::<T>(T::MaxReservedCores::get());

		let origin =
			T::AdminOrigin::try_successful_origin().map_err(|_| BenchmarkError::Weightless)?;

		#[extrinsic_call]
		_(origin as T::RuntimeOrigin, 0);

		assert_eq!(
			Reservations::<T>::get().len(),
			T::MaxReservedCores::get().saturating_sub(1) as usize
		);

		Ok(())
	}

	#[benchmark]
	fn set_lease() -> Result<(), BenchmarkError> {
		let task = 1u32;
		let until = 10u32.into();

		// Assume Leases to be almost filled for worst case
		setup_leases::<T>(T::MaxLeasedCores::get().saturating_sub(1), task, until);

		let origin =
			T::AdminOrigin::try_successful_origin().map_err(|_| BenchmarkError::Weightless)?;

		#[extrinsic_call]
		_(origin as T::RuntimeOrigin, task, until);

		assert_eq!(Leases::<T>::get().len(), T::MaxLeasedCores::get() as usize);

		Ok(())
	}

	#[benchmark]
	fn start_sales(n: Linear<0, { MAX_CORE_COUNT.into() }>) -> Result<(), BenchmarkError> {
		let config = new_config_record::<T>();
		Configuration::<T>::put(config.clone());

		let mut extra_cores = n;

		// Assume Reservations to be filled for worst case
		setup_reservations::<T>(extra_cores.min(T::MaxReservedCores::get()));
		extra_cores = extra_cores.saturating_sub(T::MaxReservedCores::get());

		// Assume Leases to be filled for worst case
		setup_leases::<T>(extra_cores.min(T::MaxLeasedCores::get()), 1, 10);
		extra_cores = extra_cores.saturating_sub(T::MaxLeasedCores::get());

		let latest_region_begin = Broker::<T>::latest_timeslice_ready_to_commit(&config);

		let initial_price = 10_000_000u32.into();

		let origin =
			T::AdminOrigin::try_successful_origin().map_err(|_| BenchmarkError::Weightless)?;

		#[extrinsic_call]
		_(origin as T::RuntimeOrigin, initial_price, extra_cores.try_into().unwrap());

		assert!(SaleInfo::<T>::get().is_some());
		let sale_start = RCBlockNumberProviderOf::<T::Coretime>::current_block_number()
			+ config.interlude_length;
		assert_last_event::<T>(
			Event::SaleInitialized {
				sale_start,
				leadin_length: 1u32.into(),
				start_price: 1_000_000_000u32.into(),
				end_price: 10_000_000u32.into(),
				region_begin: latest_region_begin + config.region_length,
				region_end: latest_region_begin + config.region_length * 2,
				ideal_cores_sold: 0,
				cores_offered: n
					.saturating_sub(T::MaxReservedCores::get())
					.saturating_sub(T::MaxLeasedCores::get())
					.try_into()
					.unwrap(),
			}
			.into(),
		);

		Ok(())
	}

	#[benchmark]
	fn purchase() -> Result<(), BenchmarkError> {
		let core = setup_and_start_sale::<T>()?;

		advance_to::<T>(2);

		let caller: T::AccountId = whitelisted_caller();
		T::Currency::set_balance(
			&caller.clone(),
			T::Currency::minimum_balance().saturating_add(10_000_000u32.into()),
		);

		#[extrinsic_call]
		_(RawOrigin::Signed(caller.clone()), 10_000_000u32.into());

		assert_eq!(SaleInfo::<T>::get().unwrap().sellout_price, Some(10_000_000u32.into()));
		assert_last_event::<T>(
			Event::Purchased {
				who: caller,
				region_id: RegionId {
					begin: SaleInfo::<T>::get().unwrap().region_begin,
					core,
					mask: CoreMask::complete(),
				},
				price: 10_000_000u32.into(),
				duration: 3u32.into(),
			}
			.into(),
		);

		Ok(())
	}

	#[benchmark]
	fn renew() -> Result<(), BenchmarkError> {
		setup_and_start_sale::<T>()?;
		let region_len = Configuration::<T>::get().unwrap().region_length;

		advance_to::<T>(2);

		let caller: T::AccountId = whitelisted_caller();
		T::Currency::set_balance(
			&caller.clone(),
			T::Currency::minimum_balance().saturating_add(20_000_000u32.into()),
		);

		let region = Broker::<T>::do_purchase(caller.clone(), 10_000_000u32.into())
			.map_err(|_| BenchmarkError::Weightless)?;

		Broker::<T>::do_assign(region, None, 1001, Final)
			.map_err(|_| BenchmarkError::Weightless)?;

		advance_to::<T>((T::TimeslicePeriod::get() * region_len.into()).try_into().ok().unwrap());

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), region.core);

		let id = PotentialRenewalId { core: region.core, when: region.begin + region_len * 2 };
		assert!(PotentialRenewals::<T>::get(id).is_some());

		Ok(())
	}

	#[benchmark]
	fn transfer() -> Result<(), BenchmarkError> {
		setup_and_start_sale::<T>()?;

		advance_to::<T>(2);

		let caller: T::AccountId = whitelisted_caller();
		T::Currency::set_balance(
			&caller.clone(),
			T::Currency::minimum_balance().saturating_add(10_000_000u32.into()),
		);

		let region = Broker::<T>::do_purchase(caller.clone(), 10_000_000u32.into())
			.map_err(|_| BenchmarkError::Weightless)?;

		let recipient: T::AccountId = account("recipient", 0, SEED);

		#[extrinsic_call]
		_(RawOrigin::Signed(caller.clone()), region, recipient.clone());

		assert_last_event::<T>(
			Event::Transferred {
				region_id: region,
				old_owner: Some(caller),
				owner: Some(recipient),
				duration: 3u32.into(),
			}
			.into(),
		);

		Ok(())
	}

	#[benchmark]
	fn partition() -> Result<(), BenchmarkError> {
		let core = setup_and_start_sale::<T>()?;

		advance_to::<T>(2);

		let caller: T::AccountId = whitelisted_caller();
		T::Currency::set_balance(
			&caller.clone(),
			T::Currency::minimum_balance().saturating_add(10_000_000u32.into()),
		);

		let region = Broker::<T>::do_purchase(caller.clone(), 10_000_000u32.into())
			.map_err(|_| BenchmarkError::Weightless)?;

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), region, 2);

		assert_last_event::<T>(
			Event::Partitioned {
				old_region_id: RegionId { begin: region.begin, core, mask: CoreMask::complete() },
				new_region_ids: (
					RegionId { begin: region.begin, core, mask: CoreMask::complete() },
					RegionId { begin: region.begin + 2, core, mask: CoreMask::complete() },
				),
			}
			.into(),
		);

		Ok(())
	}

	#[benchmark]
	fn interlace() -> Result<(), BenchmarkError> {
		let core = setup_and_start_sale::<T>()?;

		advance_to::<T>(2);

		let caller: T::AccountId = whitelisted_caller();
		T::Currency::set_balance(
			&caller.clone(),
			T::Currency::minimum_balance().saturating_add(10_000_000u32.into()),
		);

		let region = Broker::<T>::do_purchase(caller.clone(), 10_000_000u32.into())
			.map_err(|_| BenchmarkError::Weightless)?;

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), region, 0x00000_fffff_fffff_00000.into());

		assert_last_event::<T>(
			Event::Interlaced {
				old_region_id: RegionId { begin: region.begin, core, mask: CoreMask::complete() },
				new_region_ids: (
					RegionId { begin: region.begin, core, mask: 0x00000_fffff_fffff_00000.into() },
					RegionId {
						begin: region.begin,
						core,
						mask: CoreMask::complete() ^ 0x00000_fffff_fffff_00000.into(),
					},
				),
			}
			.into(),
		);

		Ok(())
	}

	#[benchmark]
	fn assign() -> Result<(), BenchmarkError> {
		let core = setup_and_start_sale::<T>()?;

		advance_to::<T>(2);

		let caller: T::AccountId = whitelisted_caller();
		T::Currency::set_balance(
			&caller.clone(),
			T::Currency::minimum_balance().saturating_add(10_000_000u32.into()),
		);

		let region = Broker::<T>::do_purchase(caller.clone(), 10_000_000u32.into())
			.map_err(|_| BenchmarkError::Weightless)?;

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), region, 1000, Provisional);

		let workplan_key = (region.begin, region.core);
		assert!(Workplan::<T>::get(workplan_key).is_some());

		assert!(Regions::<T>::get(region).is_some());

		assert_last_event::<T>(
			Event::Assigned {
				region_id: RegionId { begin: region.begin, core, mask: CoreMask::complete() },
				task: 1000,
				duration: 3u32.into(),
			}
			.into(),
		);

		Ok(())
	}

	#[benchmark]
	fn pool() -> Result<(), BenchmarkError> {
		let core = setup_and_start_sale::<T>()?;

		advance_to::<T>(2);

		let caller: T::AccountId = whitelisted_caller();
		T::Currency::set_balance(
			&caller.clone(),
			T::Currency::minimum_balance().saturating_add(10_000_000u32.into()),
		);

		let region = Broker::<T>::do_purchase(caller.clone(), 10_000_000u32.into())
			.map_err(|_| BenchmarkError::Weightless)?;

		let recipient: T::AccountId = account("recipient", 0, SEED);

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), region, recipient, Final);

		let workplan_key = (region.begin, region.core);
		assert!(Workplan::<T>::get(workplan_key).is_some());

		assert_last_event::<T>(
			Event::Pooled {
				region_id: RegionId { begin: region.begin, core, mask: CoreMask::complete() },
				duration: 3u32.into(),
			}
			.into(),
		);

		Ok(())
	}

	#[benchmark]
	fn claim_revenue(
		m: Linear<1, { new_config_record::<T>().region_length }>,
	) -> Result<(), BenchmarkError> {
		let core = setup_and_start_sale::<T>()?;

		advance_to::<T>(2);

		let caller: T::AccountId = whitelisted_caller();
		T::Currency::set_balance(
			&caller.clone(),
			T::Currency::minimum_balance().saturating_add(10_000_000u32.into()),
		);
		T::Currency::set_balance(
			&Broker::<T>::account_id(),
			T::Currency::minimum_balance().saturating_add(200_000_000u32.into()),
		);

		let region = Broker::<T>::do_purchase(caller.clone(), 10_000_000u32.into())
			.map_err(|_| BenchmarkError::Weightless)?;

		let recipient: T::AccountId = account("recipient", 0, SEED);
		T::Currency::set_balance(&recipient.clone(), T::Currency::minimum_balance());

		Broker::<T>::do_pool(region, None, recipient.clone(), Final)
			.map_err(|_| BenchmarkError::Weightless)?;

		let revenue = 10_000_000u32.into();
		InstaPoolHistory::<T>::insert(
			region.begin,
			InstaPoolHistoryRecord {
				private_contributions: 4u32.into(),
				system_contributions: 3u32.into(),
				maybe_payout: Some(revenue),
			},
		);

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), region, m);

		assert!(InstaPoolHistory::<T>::get(region.begin).is_none());
		assert_last_event::<T>(
			Event::RevenueClaimPaid {
				who: recipient,
				amount: 200_000_000u32.into(),
				next: if m < new_config_record::<T>().region_length {
					Some(RegionId {
						begin: region.begin.saturating_add(m),
						core,
						mask: CoreMask::complete(),
					})
				} else {
					None
				},
			}
			.into(),
		);

		Ok(())
	}

	#[benchmark]
	fn purchase_credit() -> Result<(), BenchmarkError> {
		setup_and_start_sale::<T>()?;

		advance_to::<T>(2);

		let caller: T::AccountId = whitelisted_caller();
		T::Currency::set_balance(
			&caller.clone(),
			T::Currency::minimum_balance().saturating_add(30_000_000u32.into()),
		);
		T::Currency::set_balance(&Broker::<T>::account_id(), T::Currency::minimum_balance());

		let region = Broker::<T>::do_purchase(caller.clone(), 10_000_000u32.into())
			.map_err(|_| BenchmarkError::Weightless)?;

		let recipient: T::AccountId = account("recipient", 0, SEED);

		Broker::<T>::do_pool(region, None, recipient, Final)
			.map_err(|_| BenchmarkError::Weightless)?;

		let beneficiary: RelayAccountIdOf<T> = account("beneficiary", 0, SEED);

		#[extrinsic_call]
		_(RawOrigin::Signed(caller.clone()), 20_000_000u32.into(), beneficiary.clone());

		assert_last_event::<T>(
			Event::CreditPurchased { who: caller, beneficiary, amount: 20_000_000u32.into() }
				.into(),
		);

		Ok(())
	}

	#[benchmark]
	fn drop_region() -> Result<(), BenchmarkError> {
		let core = setup_and_start_sale::<T>()?;
		let region_len = Configuration::<T>::get().unwrap().region_length;

		advance_to::<T>(2);

		let caller: T::AccountId = whitelisted_caller();
		T::Currency::set_balance(
			&caller.clone(),
			T::Currency::minimum_balance().saturating_add(10_000_000u32.into()),
		);

		let region = Broker::<T>::do_purchase(caller.clone(), 10_000_000u32.into())
			.map_err(|_| BenchmarkError::Weightless)?;

		advance_to::<T>(
			(T::TimeslicePeriod::get() * (region_len * 4).into()).try_into().ok().unwrap(),
		);

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), region);

		assert_last_event::<T>(
			Event::RegionDropped {
				region_id: RegionId { begin: region.begin, core, mask: CoreMask::complete() },
				duration: 3u32.into(),
			}
			.into(),
		);

		Ok(())
	}

	#[benchmark]
	fn drop_contribution() -> Result<(), BenchmarkError> {
		let core = setup_and_start_sale::<T>()?;
		let region_len = Configuration::<T>::get().unwrap().region_length;

		advance_to::<T>(2);

		let caller: T::AccountId = whitelisted_caller();
		T::Currency::set_balance(
			&caller.clone(),
			T::Currency::minimum_balance().saturating_add(10_000_000u32.into()),
		);

		let region = Broker::<T>::do_purchase(caller.clone(), 10_000_000u32.into())
			.map_err(|_| BenchmarkError::Weightless)?;

		let recipient: T::AccountId = account("recipient", 0, SEED);

		Broker::<T>::do_pool(region, None, recipient, Final)
			.map_err(|_| BenchmarkError::Weightless)?;

		advance_to::<T>(
			(T::TimeslicePeriod::get() * (region_len * 8).into()).try_into().ok().unwrap(),
		);

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), region);

		assert_last_event::<T>(
			Event::ContributionDropped {
				region_id: RegionId { begin: region.begin, core, mask: CoreMask::complete() },
			}
			.into(),
		);

		Ok(())
	}

	#[benchmark]
	fn drop_history() -> Result<(), BenchmarkError> {
		setup_and_start_sale::<T>()?;
		let when = 5u32.into();
		let revenue = 10_000_000u32.into();
		let region_len = Configuration::<T>::get().unwrap().region_length;

		advance_to::<T>(
			(T::TimeslicePeriod::get() * (region_len * 8).into()).try_into().ok().unwrap(),
		);

		let caller: T::AccountId = whitelisted_caller();
		InstaPoolHistory::<T>::insert(
			when,
			InstaPoolHistoryRecord {
				private_contributions: 4u32.into(),
				system_contributions: 3u32.into(),
				maybe_payout: Some(revenue),
			},
		);

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), when);

		assert!(InstaPoolHistory::<T>::get(when).is_none());
		assert_last_event::<T>(Event::HistoryDropped { when, revenue }.into());

		Ok(())
	}

	#[benchmark]
	fn drop_renewal() -> Result<(), BenchmarkError> {
		let core = setup_and_start_sale::<T>()?;
		let when = 5u32.into();
		let region_len = Configuration::<T>::get().unwrap().region_length;

		advance_to::<T>(
			(T::TimeslicePeriod::get() * (region_len * 3).into()).try_into().ok().unwrap(),
		);

		let id = PotentialRenewalId { core, when };
		let record = PotentialRenewalRecord {
			price: 1_000_000u32.into(),
			completion: CompletionStatus::Complete(new_schedule()),
		};
		PotentialRenewals::<T>::insert(id, record);

		let caller: T::AccountId = whitelisted_caller();

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), core, when);

		assert!(PotentialRenewals::<T>::get(id).is_none());
		assert_last_event::<T>(Event::PotentialRenewalDropped { core, when }.into());

		Ok(())
	}

	#[benchmark]
	fn request_core_count(n: Linear<0, { MAX_CORE_COUNT.into() }>) -> Result<(), BenchmarkError> {
		let admin_origin =
			T::AdminOrigin::try_successful_origin().map_err(|_| BenchmarkError::Weightless)?;

		#[extrinsic_call]
		_(admin_origin as T::RuntimeOrigin, n.try_into().unwrap());

		assert_last_event::<T>(
			Event::CoreCountRequested { core_count: n.try_into().unwrap() }.into(),
		);

		Ok(())
	}

	#[benchmark]
	fn process_core_count(n: Linear<0, { MAX_CORE_COUNT.into() }>) -> Result<(), BenchmarkError> {
		setup_and_start_sale::<T>()?;

		let core_count = n.try_into().unwrap();

		CoreCountInbox::<T>::put(core_count);

		let mut status = Status::<T>::get().ok_or(BenchmarkError::Weightless)?;

		#[block]
		{
			Broker::<T>::process_core_count(&mut status);
		}

		assert_last_event::<T>(Event::CoreCountChanged { core_count }.into());

		Ok(())
	}

	#[benchmark]
	fn process_revenue() -> Result<(), BenchmarkError> {
		setup_and_start_sale::<T>()?;

		advance_to::<T>(2);

		let caller: T::AccountId = whitelisted_caller();
		T::Currency::set_balance(
			&caller.clone(),
			T::Currency::minimum_balance().saturating_add(30_000_000u32.into()),
		);
		T::Currency::set_balance(
			&Broker::<T>::account_id(),
			T::Currency::minimum_balance().saturating_add(90_000_000u32.into()),
		);

		let timeslice_period: u32 = T::TimeslicePeriod::get().try_into().ok().unwrap();
		let multiplicator = 5;

		RevenueInbox::<T>::put(OnDemandRevenueRecord {
			until: (timeslice_period * multiplicator).into(),
			amount: 10_000_000u32.into(),
		});

		let timeslice = multiplicator - 1;
		InstaPoolHistory::<T>::insert(
			timeslice,
			InstaPoolHistoryRecord {
				private_contributions: 4u32.into(),
				system_contributions: 6u32.into(),
				maybe_payout: None,
			},
		);

		#[block]
		{
			Broker::<T>::process_revenue();
		}

		assert_last_event::<T>(
			Event::ClaimsReady {
				when: timeslice.into(),
				system_payout: 6_000_000u32.into(),
				private_payout: 4_000_000u32.into(),
			}
			.into(),
		);

		Ok(())
	}

	#[benchmark]
	fn rotate_sale(n: Linear<0, { MAX_CORE_COUNT.into() }>) -> Result<(), BenchmarkError> {
		let core_count = n.try_into().unwrap();
		let config = new_config_record::<T>();

		let now = RCBlockNumberProviderOf::<T::Coretime>::current_block_number();
		let end_price = 10_000_000u32.into();
		let commit_timeslice = Broker::<T>::latest_timeslice_ready_to_commit(&config);
		let sale = SaleInfoRecordOf::<T> {
			sale_start: now,
			leadin_length: Zero::zero(),
			end_price,
			sellout_price: None,
			region_begin: commit_timeslice,
			region_end: commit_timeslice.saturating_add(config.region_length),
			first_core: 0,
			ideal_cores_sold: 0,
			cores_offered: 0,
			cores_sold: 0,
		};

		let status = StatusRecord {
			core_count,
			private_pool_size: 0,
			system_pool_size: 0,
			last_committed_timeslice: commit_timeslice.saturating_sub(1),
			last_timeslice: Broker::<T>::current_timeslice(),
		};

		// Assume Reservations to be filled for worst case
		setup_reservations::<T>(T::MaxReservedCores::get());

		// Assume Leases to be filled for worst case
		setup_leases::<T>(T::MaxLeasedCores::get(), 1, 10);

		// Assume max auto renewals for worst case.
		(0..T::MaxAutoRenewals::get()).try_for_each(|indx| -> Result<(), BenchmarkError> {
			let task = 1000 + indx;
			let caller: T::AccountId = T::SovereignAccountOf::maybe_convert(task)
				.expect("Failed to get sovereign account");
			T::Currency::set_balance(
				&caller.clone(),
				T::Currency::minimum_balance().saturating_add(100u32.into()),
			);

			let region = Broker::<T>::do_purchase(caller.clone(), 10u32.into())
				.map_err(|_| BenchmarkError::Weightless)?;

			Broker::<T>::do_assign(region, None, task, Final)
				.map_err(|_| BenchmarkError::Weightless)?;

			Broker::<T>::do_enable_auto_renew(caller, region.core, task, None)?;

			Ok(())
		})?;

		#[block]
		{
			Broker::<T>::rotate_sale(sale.clone(), &config, &status);
		}

		assert!(SaleInfo::<T>::get().is_some());
		let sale_start = RCBlockNumberProviderOf::<T::Coretime>::current_block_number()
			+ config.interlude_length;
		assert_last_event::<T>(
			Event::SaleInitialized {
				sale_start,
				leadin_length: 1u32.into(),
				start_price: 1_000_000_000u32.into(),
				end_price: 10_000_000u32.into(),
				region_begin: sale.region_begin + config.region_length,
				region_end: sale.region_end + config.region_length,
				ideal_cores_sold: 0,
				cores_offered: n
					.saturating_sub(T::MaxReservedCores::get())
					.saturating_sub(T::MaxLeasedCores::get())
					.try_into()
					.unwrap(),
			}
			.into(),
		);

		// Make sure all cores got renewed:
		(0..T::MaxAutoRenewals::get()).for_each(|indx| {
			let task = 1000 + indx;
			let who = T::SovereignAccountOf::maybe_convert(task)
				.expect("Failed to get sovereign account");
			assert_has_event::<T>(
				Event::Renewed {
					who,
					old_core: 10 + indx as u16, // first ten cores are allocated to leases.
					core: 10 + indx as u16,
					price: 10u32.saturated_into(),
					begin: 7,
					duration: 3,
					workload: Schedule::truncate_from(vec![ScheduleItem {
						assignment: Task(task),
						mask: CoreMask::complete(),
					}]),
				}
				.into(),
			);
		});

		Ok(())
	}

	#[benchmark]
	fn process_pool() {
		let when = 10u32.into();
		let private_pool_size = 5u32.into();
		let system_pool_size = 4u32.into();

		let config = new_config_record::<T>();
		let commit_timeslice = Broker::<T>::latest_timeslice_ready_to_commit(&config);
		let mut status = StatusRecord {
			core_count: 5u16.into(),
			private_pool_size,
			system_pool_size,
			last_committed_timeslice: commit_timeslice.saturating_sub(1),
			last_timeslice: Broker::<T>::current_timeslice(),
		};

		#[block]
		{
			Broker::<T>::process_pool(when, &mut status);
		}

		assert!(InstaPoolHistory::<T>::get(when).is_some());
		assert_last_event::<T>(
			Event::HistoryInitialized { when, private_pool_size, system_pool_size }.into(),
		);
	}

	#[benchmark]
	fn process_core_schedule() {
		let timeslice = 10u32.into();
		let core = 5u16.into();
		let rc_begin = 1u32.into();

		Workplan::<T>::insert((timeslice, core), new_schedule());

		#[block]
		{
			Broker::<T>::process_core_schedule(timeslice, rc_begin, core);
		}

		assert_eq!(Workload::<T>::get(core).len(), CORE_MASK_BITS);

		let mut assignment: Vec<(CoreAssignment, PartsOf57600)> = vec![];
		for i in 0..CORE_MASK_BITS {
			assignment.push((CoreAssignment::Task(i.try_into().unwrap()), 57600));
		}
		assert_last_event::<T>(Event::CoreAssigned { core, when: rc_begin, assignment }.into());
	}

	#[benchmark]
	fn request_revenue_info_at() {
		let current_timeslice = Broker::<T>::current_timeslice();
		let rc_block = T::TimeslicePeriod::get() * current_timeslice.into();

		#[block]
		{
			T::Coretime::request_revenue_info_at(rc_block);
		}
	}

	#[benchmark]
	fn notify_core_count() -> Result<(), BenchmarkError> {
		let admin_origin =
			T::AdminOrigin::try_successful_origin().map_err(|_| BenchmarkError::Weightless)?;

		#[extrinsic_call]
		_(admin_origin as T::RuntimeOrigin, 100);

		assert!(CoreCountInbox::<T>::take().is_some());
		Ok(())
	}

	#[benchmark]
	fn notify_revenue() -> Result<(), BenchmarkError> {
		let admin_origin =
			T::AdminOrigin::try_successful_origin().map_err(|_| BenchmarkError::Weightless)?;

		#[extrinsic_call]
		_(
			admin_origin as T::RuntimeOrigin,
			OnDemandRevenueRecord { until: 100u32.into(), amount: 100_000_000u32.into() },
		);

		assert!(RevenueInbox::<T>::take().is_some());
		Ok(())
	}

	#[benchmark]
	fn do_tick_base() -> Result<(), BenchmarkError> {
		setup_and_start_sale::<T>()?;

		advance_to::<T>(5);

		let mut status = Status::<T>::get().unwrap();
		status.last_committed_timeslice = 3;
		Status::<T>::put(&status);

		#[block]
		{
			Broker::<T>::do_tick();
		}

		let updated_status = Status::<T>::get().unwrap();
		assert_eq!(status, updated_status);

		Ok(())
	}

	#[benchmark]
	fn swap_leases() -> Result<(), BenchmarkError> {
		let admin_origin =
			T::AdminOrigin::try_successful_origin().map_err(|_| BenchmarkError::Weightless)?;

		// Add two leases in `Leases`
		let n = (T::MaxLeasedCores::get() / 2) as usize;
		let mut leases = vec![LeaseRecordItem { task: 1, until: 10u32.into() }; n];
		leases.extend(vec![LeaseRecordItem { task: 2, until: 20u32.into() }; n]);
		Leases::<T>::put(BoundedVec::try_from(leases).unwrap());

		#[extrinsic_call]
		_(admin_origin as T::RuntimeOrigin, 1, 2);

		Ok(())
	}

	#[benchmark]
	fn enable_auto_renew() -> Result<(), BenchmarkError> {
		let _core = setup_and_start_sale::<T>()?;

		advance_to::<T>(2);

		// We assume max auto renewals for worst case.
		(0..T::MaxAutoRenewals::get() - 1).try_for_each(|indx| -> Result<(), BenchmarkError> {
			let task = 1000 + indx;
			let caller: T::AccountId = T::SovereignAccountOf::maybe_convert(task)
				.expect("Failed to get sovereign account");
			T::Currency::set_balance(
				&caller.clone(),
				T::Currency::minimum_balance().saturating_add(100u32.into()),
			);

			let region = Broker::<T>::do_purchase(caller.clone(), 10u32.into())
				.map_err(|_| BenchmarkError::Weightless)?;

			Broker::<T>::do_assign(region, None, task, Final)
				.map_err(|_| BenchmarkError::Weightless)?;

			Broker::<T>::do_enable_auto_renew(caller, region.core, task, Some(7))?;

			Ok(())
		})?;

		let caller: T::AccountId =
			T::SovereignAccountOf::maybe_convert(2001).expect("Failed to get sovereign account");
		T::Currency::set_balance(
			&caller.clone(),
			T::Currency::minimum_balance().saturating_add(100u32.into()),
		);

		// The region for which we benchmark enable auto renew.
		let region = Broker::<T>::do_purchase(caller.clone(), 10u32.into())
			.map_err(|_| BenchmarkError::Weightless)?;
		Broker::<T>::do_assign(region, None, 2001, Final)
			.map_err(|_| BenchmarkError::Weightless)?;

		// The most 'intensive' path is when we renew the core upon enabling auto-renewal.
		// Therefore, we advance to next bulk sale:
		advance_to::<T>(6);

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), region.core, 2001, None);

		assert_last_event::<T>(Event::AutoRenewalEnabled { core: region.core, task: 2001 }.into());
		// Make sure we indeed renewed:
		assert!(PotentialRenewals::<T>::get(PotentialRenewalId {
			core: region.core,
			when: 10 // region end after renewal
		})
		.is_some());

		Ok(())
	}

	#[benchmark]
	fn disable_auto_renew() -> Result<(), BenchmarkError> {
		let _core = setup_and_start_sale::<T>()?;

		advance_to::<T>(2);

		// We assume max auto renewals for worst case.
		(0..T::MaxAutoRenewals::get() - 1).try_for_each(|indx| -> Result<(), BenchmarkError> {
			let task = 1000 + indx;
			let caller: T::AccountId = T::SovereignAccountOf::maybe_convert(task)
				.expect("Failed to get sovereign account");
			T::Currency::set_balance(
				&caller.clone(),
				T::Currency::minimum_balance().saturating_add(100u32.into()),
			);

			let region = Broker::<T>::do_purchase(caller.clone(), 10u32.into())
				.map_err(|_| BenchmarkError::Weightless)?;

			Broker::<T>::do_assign(region, None, task, Final)
				.map_err(|_| BenchmarkError::Weightless)?;

			Broker::<T>::do_enable_auto_renew(caller, region.core, task, Some(7))?;

			Ok(())
		})?;

		let caller: T::AccountId =
			T::SovereignAccountOf::maybe_convert(1000).expect("Failed to get sovereign account");
		#[extrinsic_call]
		_(RawOrigin::Signed(caller), _core, 1000);

		assert_last_event::<T>(Event::AutoRenewalDisabled { core: _core, task: 1000 }.into());

		Ok(())
	}

	#[benchmark]
	fn on_new_timeslice() -> Result<(), BenchmarkError> {
		setup_and_start_sale::<T>()?;

		advance_to::<T>(2);

		let caller: T::AccountId = whitelisted_caller();
		T::Currency::set_balance(
			&caller.clone(),
			T::Currency::minimum_balance().saturating_add(u32::MAX.into()),
		);

		let _region = Broker::<T>::do_purchase(caller.clone(), (u32::MAX / 2).into())
			.map_err(|_| BenchmarkError::Weightless)?;

		let timeslice = Broker::<T>::current_timeslice();

		#[block]
		{
			T::Coretime::on_new_timeslice(timeslice);
		}

		Ok(())
	}

	// Implements a test for each benchmark. Execute with:
	// `cargo test -p pallet-broker --features runtime-benchmarks`.
	impl_benchmark_test_suite!(Pallet, crate::mock::new_test_ext(), crate::mock::Test);
}
