import * as SliderPrimitive from "@radix-ui/react-slider";

export default function Slider({ label, value, min = 0, max = 100, step = 1, disabled, onChange }: {
  label: string;
  value: number;
  min?: number;
  max?: number;
  step?: number;
  disabled?: boolean;
  onChange(value: number): void;
}) {
  return <SliderPrimitive.Root
    className="range-slider"
    value={[value]}
    min={min}
    max={max}
    step={step}
    disabled={disabled}
    onValueChange={([next]) => onChange(next)}
  >
    <SliderPrimitive.Track className="range-slider-track">
      <SliderPrimitive.Range className="range-slider-fill" />
    </SliderPrimitive.Track>
    <SliderPrimitive.Thumb className="range-slider-thumb" aria-label={label} />
  </SliderPrimitive.Root>;
}
