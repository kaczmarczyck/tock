// Licensed under the Apache License, Version 2.0 or the MIT License.
// SPDX-License-Identifier: Apache-2.0 OR MIT
// Copyright Tock Contributors 2022.

#![no_std]

use cortexm4::{CortexM4, CortexMVariant};

pub use stm32f4xx::{
    adc, can, chip, dbg, dma, exti, gpio, nvic, rcc, spi, syscfg, tim2, trng, usart,
};

pub mod can_registers;
pub mod interrupt_service;
pub mod stm32f429zi_nvic;
pub mod trng_registers;

// STM32F42xxx and STM32F43xxx has total of 91 interrupts
#[cfg_attr(all(target_arch = "arm", target_os = "none"), link_section = ".irqs")]
// `used` ensures that the symbol is kept until the final binary. However, as of
// May 2020, due to the compilation process, there must be some other compiled
// code here to make sure the object file is kept around. That means at minimum
// there must be an `init()` function here so that compiler does not just ignore
// the `IRQS` object. See https://github.com/rust-lang/rust/issues/56639 for a
// related discussion.
#[cfg_attr(all(target_arch = "arm", target_os = "none"), used)]
pub static IRQS: [unsafe extern "C" fn(); 91] = [
    CortexM4::GENERIC_ISR, // WWDG (0)
    CortexM4::GENERIC_ISR, // PVD (1)
    CortexM4::GENERIC_ISR, // TAMP_STAMP (2)
    CortexM4::GENERIC_ISR, // RTC_WKUP (3)
    CortexM4::GENERIC_ISR, // FLASH (4)
    CortexM4::GENERIC_ISR, // RCC (5)
    CortexM4::GENERIC_ISR, // EXTI0 (6)
    CortexM4::GENERIC_ISR, // EXTI1 (7)
    CortexM4::GENERIC_ISR, // EXTI2 (8)
    CortexM4::GENERIC_ISR, // EXTI3 (9)
    CortexM4::GENERIC_ISR, // EXTI4 (10)
    CortexM4::GENERIC_ISR, // DMA1_Stream0 (11)
    CortexM4::GENERIC_ISR, // DMA1_Stream1 (12)
    CortexM4::GENERIC_ISR, // DMA1_Stream2 (13)
    CortexM4::GENERIC_ISR, // DMA1_Stream3 (14)
    CortexM4::GENERIC_ISR, // DMA1_Stream4 (15)
    CortexM4::GENERIC_ISR, // DMA1_Stream5 (16)
    CortexM4::GENERIC_ISR, // DMA1_Stream6 (17)
    CortexM4::GENERIC_ISR, // ADC (18)
    CortexM4::GENERIC_ISR, // CAN1_TX (19)
    CortexM4::GENERIC_ISR, // CAN1_RX0 (20)
    CortexM4::GENERIC_ISR, // CAN1_RX1 (21)
    CortexM4::GENERIC_ISR, // CAN1_SCE (22)
    CortexM4::GENERIC_ISR, // EXTI9_5 (23)
    CortexM4::GENERIC_ISR, // TIM1_BRK_TIM9 (24)
    CortexM4::GENERIC_ISR, // TIM1_UP_TIM10 (25)
    CortexM4::GENERIC_ISR, // TIM1_TRG_COM_TIM11 (26)
    CortexM4::GENERIC_ISR, // TIM1_CC (27)
    CortexM4::GENERIC_ISR, // TIM2 (28)
    CortexM4::GENERIC_ISR, // TIM3 (29)
    CortexM4::GENERIC_ISR, // TIM4 (30)
    CortexM4::GENERIC_ISR, // I2C1_EV (31)
    CortexM4::GENERIC_ISR, // I2C1_ER (32)
    CortexM4::GENERIC_ISR, // I2C2_EV (33)
    CortexM4::GENERIC_ISR, // I2C2_ER (34)
    CortexM4::GENERIC_ISR, // SPI1 (35)
    CortexM4::GENERIC_ISR, // SPI2 (36)
    CortexM4::GENERIC_ISR, // USART1 (37)
    CortexM4::GENERIC_ISR, // USART2 (38)
    CortexM4::GENERIC_ISR, // USART3 (39)
    CortexM4::GENERIC_ISR, // EXTI15_10 (40)
    CortexM4::GENERIC_ISR, // RTC_Alarm (41)
    CortexM4::GENERIC_ISR, // OTG_FS_WKUP (42)
    CortexM4::GENERIC_ISR, // TIM8_BRK_TIM12 (43)
    CortexM4::GENERIC_ISR, // TIM8_UP_TIM13 (44)
    CortexM4::GENERIC_ISR, // TIM8_TRG_COM_TIM14 (45)
    CortexM4::GENERIC_ISR, // TIM8_CC (46)
    CortexM4::GENERIC_ISR, // DMA1_Stream7 (47)
    CortexM4::GENERIC_ISR, // FMC (48)
    CortexM4::GENERIC_ISR, // SDIO (49)
    CortexM4::GENERIC_ISR, // TIM5 (50)
    CortexM4::GENERIC_ISR, // SPI3 (51)
    CortexM4::GENERIC_ISR, // UART4 (52)
    CortexM4::GENERIC_ISR, // UART5 (53)
    CortexM4::GENERIC_ISR, // TIM6_DAC (54)
    CortexM4::GENERIC_ISR, // TIM7 (55)
    CortexM4::GENERIC_ISR, // DMA2_Stream0 (56)
    CortexM4::GENERIC_ISR, // DMA2_Stream1 (57)
    CortexM4::GENERIC_ISR, // DMA2_Stream2 (58)
    CortexM4::GENERIC_ISR, // DMA2_Stream3 (59)
    CortexM4::GENERIC_ISR, // DMA2_Stream4 (60)
    CortexM4::GENERIC_ISR, // ETH (61)
    CortexM4::GENERIC_ISR, // ETH_WKUP (62)
    CortexM4::GENERIC_ISR, // CAN2_TX (63)
    CortexM4::GENERIC_ISR, // CAN2_RX0 (64)
    CortexM4::GENERIC_ISR, // CAN2_RX1 (65)
    CortexM4::GENERIC_ISR, // CAN2_SCE (66)
    CortexM4::GENERIC_ISR, // OTG_FS (67)
    CortexM4::GENERIC_ISR, // DMA2_Stream5 (68)
    CortexM4::GENERIC_ISR, // DMA2_Stream6 (69)
    CortexM4::GENERIC_ISR, // DMA2_Stream7 (70)
    CortexM4::GENERIC_ISR, // USART6 (71)
    CortexM4::GENERIC_ISR, // I2C3_EV (72)
    CortexM4::GENERIC_ISR, // I2C3_ER (73)
    CortexM4::GENERIC_ISR, // OTG_HS_EP1_OUT (74)
    CortexM4::GENERIC_ISR, // OTG_HS_EP1_IN (75)
    CortexM4::GENERIC_ISR, // OTG_HS_WKUP (76)
    CortexM4::GENERIC_ISR, // OTG_HS (77)
    CortexM4::GENERIC_ISR, // DCMI (78)
    CortexM4::GENERIC_ISR, // CRYP (79)
    CortexM4::GENERIC_ISR, // HASH_RNG (80)
    CortexM4::GENERIC_ISR, // FPU (81)
    CortexM4::GENERIC_ISR, // USART7 (82)
    CortexM4::GENERIC_ISR, // USART8 (83)
    CortexM4::GENERIC_ISR, // SPI4 (84)
    CortexM4::GENERIC_ISR, // SPI5 (85)
    CortexM4::GENERIC_ISR, // SPI6 (86)
    CortexM4::GENERIC_ISR, // SAI1 (87)
    CortexM4::GENERIC_ISR, // LCD-TFT (88)
    CortexM4::GENERIC_ISR, // LCD-TFT (89)
    CortexM4::GENERIC_ISR, // DMA2D(90)
];

pub unsafe fn init() {
    stm32f4xx::init();
}
