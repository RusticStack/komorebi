import * as React from "react";
import { cva, type VariantProps } from "class-variance-authority";
import { cn } from "../../lib/utils";

const buttonVariants = cva(
  "inline-flex h-10 items-center justify-center gap-2 rounded-md border text-sm font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--komorebi-ring)] disabled:pointer-events-none disabled:opacity-50",
  {
    defaultVariants: {
      size: "default",
      variant: "default",
    },
    variants: {
      size: {
        default: "px-4",
        icon: "w-10 px-0",
        sm: "h-9 px-3",
      },
      variant: {
        default:
          "border-[var(--komorebi-line)] bg-[var(--komorebi-accent)] text-white hover:bg-[var(--komorebi-accent-strong)]",
        ghost:
          "border-transparent bg-transparent text-[var(--komorebi-ink)] hover:bg-[rgb(31_35_32_/_7%)]",
        outline:
          "border-[var(--komorebi-line)] bg-[rgb(255_255_255_/_70%)] text-[var(--komorebi-ink)] hover:bg-white",
        subtle:
          "border-[var(--komorebi-line)] bg-[var(--komorebi-accent-soft)] text-[var(--komorebi-accent-strong)] hover:bg-[rgb(207_225_208_/_95%)]",
      },
    },
  },
);

export type ButtonProps = React.ButtonHTMLAttributes<HTMLButtonElement> &
  VariantProps<typeof buttonVariants>;

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, size, variant, ...props }, ref) => (
    <button
      className={cn(buttonVariants({ size, variant }), className)}
      ref={ref}
      {...props}
    />
  ),
);

Button.displayName = "Button";
