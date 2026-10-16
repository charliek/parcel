import { ChevronDown } from "lucide-react"
import * as React from "react"
import { cn } from "@/lib/utils"

export type NativeSelectProps = React.SelectHTMLAttributes<HTMLSelectElement>

/** A styled native <select>: fully keyboard and screen-reader friendly, works on mobile. */
const NativeSelect = React.forwardRef<HTMLSelectElement, NativeSelectProps>(
  ({ className, children, ...props }, ref) => (
    <div className="relative">
      <select
        ref={ref}
        className={cn(
          "flex h-9 w-full appearance-none rounded-md border border-input bg-background py-1.5 pr-9 pl-3 text-sm shadow-xs ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50",
          className
        )}
        {...props}
      >
        {children}
      </select>
      <ChevronDown
        className="pointer-events-none absolute top-1/2 right-3 h-4 w-4 -translate-y-1/2 text-muted-foreground"
        aria-hidden="true"
      />
    </div>
  )
)
NativeSelect.displayName = "NativeSelect"

export { NativeSelect }
