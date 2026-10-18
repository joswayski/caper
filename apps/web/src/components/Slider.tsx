import * as SliderPrimitive from "@radix-ui/react-slider";
import { useRef } from "react";
import { playSliderTick } from "../audio/effects";

export default function Slider({
  label,
  value,
  min = 0,
  max = 100,
  step = 1,
  defaultValue,
  disabled,
  onChange,
}: {
  label: string;
  value: number;
  min?: number;
  max?: number;
  step?: number;
  defaultValue: number;
  disabled?: boolean;
  onChange(value: number): void;
}) {
  const pointerDown = useRef(false);
  const fraction = (defaultValue - min) / (max - min);
  return (
    <SliderPrimitive.Root
      className="range-slider"
      value={[value]}
      min={min}
      max={max}
      step={step}
      disabled={disabled}
      onPointerDownCapture={() => {
        pointerDown.current = true;
      }}
      onPointerUpCapture={() => {
        pointerDown.current = false;
      }}
      onPointerCancel={() => {
        pointerDown.current = false;
      }}
      onLostPointerCapture={() => {
        pointerDown.current = false;
      }}
      onKeyDownCapture={() => {
        pointerDown.current = false;
      }}
      onValueChange={([next]) => {
        // Include at least one step so narrow ranges (e.g. brush size) have a magnet too.
        const snapped =
          pointerDown.current && Math.abs(next - defaultValue) <= Math.max(step, (max - min) * 0.02)
            ? defaultValue
            : next;
        playSliderTick(max === min ? 0 : (snapped - min) / (max - min));
        onChange(snapped);
      }}
    >
      <SliderPrimitive.Track className="range-slider-track">
        <SliderPrimitive.Range className="range-slider-fill" />
      </SliderPrimitive.Track>
      <span
        className="range-slider-default"
        aria-hidden="true"
        style={{ left: `calc(${fraction * 100}% + ${0.5 - fraction} * var(--slider-thumb-size))` }}
      />
      <SliderPrimitive.Thumb className="range-slider-thumb" aria-label={label} />
    </SliderPrimitive.Root>
  );
}
