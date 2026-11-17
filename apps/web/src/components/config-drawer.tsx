/*
Copyright (C) 2023-2026 QuantumNous

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, either version 3 of the
License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program. If not, see <https://www.gnu.org/licenses/>.

For commercial licensing, please contact support@quantumnous.com
*/
import { Radio as RadioPrimitive } from '@base-ui/react/radio'
import { RadioGroup as Radio } from '@base-ui/react/radio-group'
import {
  ChevronRight,
  CircleCheck,
  Palette,
  RotateCcw,
  Settings2,
} from 'lucide-react'
import { useId, type SVGProps } from 'react'
import { useTranslation } from 'react-i18next'

import { IconDir } from '@/assets/custom/icon-dir'
import { IconLayoutCompact } from '@/assets/custom/icon-layout-compact'
import { IconLayoutDefault } from '@/assets/custom/icon-layout-default'
import { IconLayoutFull } from '@/assets/custom/icon-layout-full'
import { IconSidebarFloating } from '@/assets/custom/icon-sidebar-floating'
import { IconSidebarInset } from '@/assets/custom/icon-sidebar-inset'
import { IconSidebarSidebar } from '@/assets/custom/icon-sidebar-sidebar'
import { IconThemeDark } from '@/assets/custom/icon-theme-dark'
import { IconThemeLight } from '@/assets/custom/icon-theme-light'
import { IconThemeSystem } from '@/assets/custom/icon-theme-system'
import {
  sideDrawerContentClassName,
  sideDrawerFooterClassName,
  sideDrawerFormClassName,
  sideDrawerHeaderClassName,
} from '@/components/drawer-layout'
import { Button } from '@/components/ui/button'
import {
  Sheet,
  SheetClose,
  SheetContent,
  SheetDescription,
  SheetFooter,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from '@/components/ui/sheet'
import { useDirection } from '@/context/direction-provider'
import { type Collapsible, useLayout } from '@/context/layout-provider'
import { useThemeCustomization } from '@/context/theme-customization-provider'
import { useTheme } from '@/context/theme-provider'
import {
  type ContentLayout,
  THEME_PRESETS,
  type ThemeFont,
  type ThemePreset,
  type ThemeRadius,
  type ThemeScale,
} from '@/lib/theme-customization'
import { cn } from '@/lib/utils'

import { useSidebar } from './ui/sidebar'

const Item = RadioPrimitive.Root

export function ConfigDrawer({
  sidebarAction = false,
}: {
  sidebarAction?: boolean
}) {
  const { t } = useTranslation()
  const { setOpen } = useSidebar()
  const { resetDir } = useDirection()
  const { resetTheme } = useTheme()
  const { resetLayout } = useLayout()
  const { resetCustomization } = useThemeCustomization()

  const handleReset = () => {
    setOpen(true)
    resetDir()
    resetTheme()
    resetLayout()
    resetCustomization()
  }

  return (
    <Sheet>
      <SheetTrigger
        render={
          <Button
            size={sidebarAction ? 'default' : 'icon'}
            variant='ghost'
            aria-label={t('Open theme settings')}
            aria-describedby='config-drawer-description'
            className={cn(
              sidebarAction
                ? 'h-9 min-w-0 flex-1 justify-start gap-2 rounded-md px-2 text-sm font-medium group-data-[collapsible=icon]:size-8! group-data-[collapsible=icon]:justify-center'
                : 'max-md:hidden'
            )}
          />
        }
      >
        {sidebarAction ? (
          <>
            <Settings2 className='size-4 shrink-0' aria-hidden='true' />
            <span className='truncate group-data-[collapsible=icon]:hidden'>
              {t('Appearance')}
            </span>
            <ChevronRight
              className='text-muted-foreground ms-auto size-3.5 group-data-[collapsible=icon]:hidden'
              aria-hidden='true'
            />
          </>
        ) : (
          <Palette className='size-[1.2rem]' aria-hidden='true' />
        )}
      </SheetTrigger>
      <SheetContent className={sideDrawerContentClassName('sm:max-w-lg')}>
        <SheetHeader className={sideDrawerHeaderClassName()}>
          <SheetTitle>{t('Appearance')}</SheetTitle>
          <SheetDescription id='config-drawer-description'>
            {t('Changes apply instantly and are saved in this browser.')}
          </SheetDescription>
        </SheetHeader>
        <div className={sideDrawerFormClassName('gap-8 py-6 sm:py-7')}>
          <PresetConfig />
          <ThemeConfig />
          <FontConfig />
          <RadiusConfig />
          <ScaleConfig />
          <details className='group border-border/70 border-t pt-6'>
            <summary className='flex min-h-11 cursor-pointer list-none items-center justify-between gap-3 text-sm font-medium [&::-webkit-details-marker]:hidden'>
              {t('Layout')}
              <ChevronRight
                className='size-4 transition-transform group-open:rotate-90 motion-reduce:transition-none'
                aria-hidden='true'
              />
            </summary>
            <div className='mt-5 space-y-8'>
              <SidebarConfig />
              <LayoutConfig />
              <ContentLayoutConfig />
              <DirConfig />
            </div>
          </details>
        </div>
        <SheetFooter className={sideDrawerFooterClassName()}>
          <Button
            variant='ghost'
            onClick={handleReset}
            aria-label={t('Reset all settings to default values')}
          >
            <RotateCcw className='size-4' aria-hidden='true' />
            {t('Reset appearance')}
          </Button>
          <SheetClose render={<Button />}>{t('Done')}</SheetClose>
        </SheetFooter>
      </SheetContent>
    </Sheet>
  )
}

function SectionTitle(props: {
  title: string
  showReset?: boolean
  onReset?: () => void
  className?: string
}) {
  const { t } = useTranslation()

  return (
    <div
      className={cn(
        'text-foreground mb-3 flex min-h-8 items-center justify-between gap-2 text-sm font-medium',
        props.className
      )}
    >
      {props.title}
      {props.showReset && props.onReset && (
        <Button
          size='icon'
          variant='ghost'
          className='-my-1.5 -me-2 min-h-11 min-w-11'
          onClick={props.onReset}
          aria-label={t('Reset')}
        >
          <RotateCcw className='size-3.5' aria-hidden='true' />
        </Button>
      )}
    </div>
  )
}

function RadioGroupItem(props: {
  item: {
    value: string
    label: string
    icon: (props: SVGProps<SVGSVGElement>) => React.ReactElement
  }
  isTheme?: boolean
}) {
  const { t } = useTranslation()
  const descriptionId = useId()
  const isTheme = props.isTheme ?? false
  return (
    <Item
      value={props.item.value}
      className='group min-w-0 outline-none'
      aria-label={t('Select {{label}}', {
        label: props.item.label.toLowerCase(),
      })}
      aria-describedby={descriptionId}
    >
      <div
        className={cn(
          'ring-border/70 bg-card relative overflow-hidden rounded-lg ring-[1px] transition-colors motion-reduce:transition-none',
          'group-data-checked:ring-primary group-data-checked:bg-primary/5',
          'group-focus-visible:ring-2'
        )}
        aria-hidden='true'
      >
        <CircleCheck
          className={cn(
            'fill-primary stroke-primary-foreground size-4',
            'group-data-unchecked:hidden',
            'absolute top-2 right-2'
          )}
          aria-hidden='true'
        />
        <props.item.icon
          className={cn(
            !isTheme &&
              'stroke-primary fill-primary group-data-unchecked:stroke-muted-foreground group-data-unchecked:fill-muted-foreground'
          )}
          aria-hidden='true'
        />
      </div>
      <div className='mt-1 text-xs' id={descriptionId}>
        {props.item.label}
      </div>
    </Item>
  )
}

function ThemeConfig() {
  const { t } = useTranslation()
  const { defaultTheme, theme, setTheme } = useTheme()
  return (
    <div>
      <SectionTitle
        title={t('Theme')}
        showReset={theme !== defaultTheme}
        onReset={() => setTheme(defaultTheme)}
      />
      <Radio
        value={theme}
        onValueChange={setTheme}
        className='grid w-full max-w-md grid-cols-3 gap-4'
        aria-label={t('Select theme preference')}
        aria-describedby='theme-description'
      >
        {[
          { value: 'system', label: t('System'), icon: IconThemeSystem },
          { value: 'light', label: t('Light'), icon: IconThemeLight },
          { value: 'dark', label: t('Dark'), icon: IconThemeDark },
        ].map((item) => (
          <RadioGroupItem key={item.value} item={item} isTheme />
        ))}
      </Radio>
      <div id='theme-description' className='sr-only'>
        {t('Choose between system preference, light mode, or dark mode')}
      </div>
    </div>
  )
}

function PresetConfig() {
  const { t } = useTranslation()
  const { defaults, customization, setPreset } = useThemeCustomization()
  return (
    <div>
      <SectionTitle
        title={t('Color preset')}
        showReset={customization.preset !== defaults.preset}
        onReset={() => setPreset(defaults.preset)}
      />
      <Radio
        value={customization.preset}
        onValueChange={(v) => setPreset(v as ThemePreset)}
        className='grid w-full grid-cols-2 gap-x-4 gap-y-5'
        aria-label={t('Select color preset')}
      >
        {THEME_PRESETS.map((preset) => (
          <Item
            key={preset.value}
            value={preset.value}
            className='group flex flex-col items-stretch outline-none'
            aria-label={t(`preset.${preset.value}`)}
          >
            <div
              className={cn(
                'ring-border/70 bg-card relative h-12 rounded-lg ring-[1px] transition-colors motion-reduce:transition-none',
                'group-data-checked:ring-primary group-data-checked:bg-primary/5',
                'group-focus-visible:ring-2',
                'group-hover:ring-primary/60'
              )}
            >
              <div
                aria-hidden='true'
                className='absolute inset-0 flex overflow-hidden rounded-lg'
              >
                {preset.swatches.map((color) => (
                  <span
                    key={color}
                    className='min-w-0 flex-1'
                    style={{ backgroundColor: color }}
                  />
                ))}
              </div>
              <CircleCheck
                className={cn(
                  'fill-primary stroke-primary-foreground absolute top-2 right-2 z-10 size-4',
                  'group-data-unchecked:hidden'
                )}
                aria-hidden='true'
              />
            </div>
            <div className='mt-2 flex min-w-0 flex-wrap items-center justify-between gap-x-2 gap-y-1 text-start text-xs'>
              <span>{t(`preset.${preset.value}`)}</span>
              {preset.value === defaults.preset && (
                <span className='text-muted-foreground'>{t('Default')}</span>
              )}
            </div>
          </Item>
        ))}
      </Radio>
    </div>
  )
}

/**
 * Font options shown in the theme drawer.
 *
 * Each option renders a live "Aa" preview in the font it represents.
 * `Auto` deliberately leaves `fontFamily` undefined so the preview inherits
 * the currently active body font — that way the user sees what `Auto` will
 * actually look like for the active preset (Ant → serif glyphs,
 * everything else → sans glyphs) without us having to duplicate the
 * preset-default mapping in the UI.
 */
const FONT_OPTIONS: {
  value: ThemeFont
  label: string
  // CSS font-family applied to the "Aa" preview. `undefined` = inherit
  // from the current theme (used by the `default` option).
  preview?: string
}[] = [
  { value: 'default', label: 'Auto', preview: undefined },
  { value: 'sans', label: 'Sans', preview: 'var(--font-sans)' },
  { value: 'serif', label: 'Serif', preview: 'var(--font-serif)' },
]

function FontConfig() {
  const { t } = useTranslation()
  const { defaults, customization, setFont } = useThemeCustomization()
  return (
    <div>
      <SectionTitle
        title={t('Font')}
        showReset={customization.font !== defaults.font}
        onReset={() => setFont(defaults.font)}
      />
      <Radio
        value={customization.font}
        onValueChange={(v) => setFont(v as ThemeFont)}
        className='grid w-full grid-cols-3 gap-4'
        aria-label={t('Select body font')}
      >
        {FONT_OPTIONS.map((option) => (
          <Item
            key={option.value}
            value={option.value}
            className='group flex flex-col items-stretch outline-none'
            aria-label={
              option.value === 'default' ? t('System default') : t(option.label)
            }
          >
            <div
              className={cn(
                'ring-border/70 bg-card relative h-12 rounded-lg ring-[1px] transition-colors motion-reduce:transition-none',
                'group-data-checked:ring-primary group-data-checked:bg-primary/5',
                'group-focus-visible:ring-2',
                'group-hover:ring-primary/60'
              )}
            >
              <CircleCheck
                className={cn(
                  'fill-primary stroke-primary-foreground absolute top-2 right-2 z-10 size-4',
                  'group-data-unchecked:hidden'
                )}
                aria-hidden='true'
              />
              <span
                aria-hidden='true'
                className='text-foreground absolute inset-0 flex items-center justify-center text-lg leading-none font-medium'
                style={
                  option.preview
                    ? { fontFamily: option.preview }
                    : // `font: inherit` defers to the active theme so the
                      // "Auto" tile previews what the resolved font will be.
                      { font: 'inherit', fontSize: '1.125rem' }
                }
              >
                Aa
              </span>
            </div>
            <div className='mt-1.5 text-center text-xs'>{t(option.label)}</div>
          </Item>
        ))}
      </Radio>
    </div>
  )
}

const RADIUS_OPTIONS: {
  value: ThemeRadius
  label: string
  // CSS border-radius value used to render the visual preview corner.
  preview: string
}[] = [
  { value: 'default', label: 'Auto', preview: '1rem' },
  { value: 'none', label: '0', preview: '0' },
  { value: 'sm', label: '0.3', preview: '0.3rem' },
  { value: 'md', label: '0.5', preview: '0.5rem' },
  { value: 'lg', label: '0.75', preview: '0.75rem' },
  { value: 'xl', label: '1.0', preview: '1rem' },
]

function RadiusConfig() {
  const { t } = useTranslation()
  const { defaults, customization, setRadius } = useThemeCustomization()
  return (
    <div>
      <SectionTitle
        title={t('Border radius')}
        showReset={customization.radius !== defaults.radius}
        onReset={() => setRadius(defaults.radius)}
      />
      <Radio
        value={customization.radius}
        onValueChange={(v) => setRadius(v as ThemeRadius)}
        className='grid w-full grid-cols-3 gap-3 sm:grid-cols-6'
        aria-label={t('Select border radius')}
      >
        {RADIUS_OPTIONS.map((option) => (
          <Item
            key={option.value}
            value={option.value}
            className='group flex flex-col items-stretch outline-none'
            aria-label={
              option.value === 'default' ? t('System default') : option.label
            }
          >
            <div
              className={cn(
                'ring-border/70 bg-card relative h-12 rounded-lg ring-[1px] transition-colors motion-reduce:transition-none',
                'group-data-checked:ring-primary group-data-checked:bg-primary/5',
                'group-focus-visible:ring-2',
                'group-hover:ring-primary/60'
              )}
            >
              <CircleCheck
                className={cn(
                  'fill-primary stroke-primary-foreground absolute top-2 right-2 z-10 size-4',
                  'group-data-unchecked:hidden'
                )}
                aria-hidden='true'
              />
              <span
                aria-hidden='true'
                className='border-foreground/70 absolute top-2.5 left-2.5 size-3.5 border-t-[1.5px] border-l-[1.5px]'
                style={{ borderTopLeftRadius: option.preview }}
              />
            </div>
            <div className='mt-1.5 text-center text-xs'>
              {option.value === 'default' ? t('Auto') : option.label}
            </div>
          </Item>
        ))}
      </Radio>
    </div>
  )
}

/**
 * Visual preview rows for the density preset. Each row's height represents
 * the relative line-height density (compact = tight rows, comfortable = wide).
 */
function ScalePreview(props: { rows: number; rowGap: string }) {
  return (
    <div
      aria-hidden='true'
      className='absolute inset-2.5 flex flex-col justify-center'
      style={{ gap: props.rowGap }}
    >
      {Array.from({ length: props.rows }, (_, index) => 85 - index * 10).map(
        (width) => (
          <span
            key={width}
            className='bg-foreground/60 block h-[2px] rounded-full'
            style={{ width: `${width}%` }}
          />
        )
      )}
    </div>
  )
}

function ScaleConfig() {
  const { t } = useTranslation()
  const { defaults, customization, setScale } = useThemeCustomization()
  const scaleOptions: {
    value: ThemeScale
    label: string
    rows: number
    rowGap: string
  }[] = [
    { value: 'sm', label: t('Compact'), rows: 4, rowGap: '3px' },
    { value: 'default', label: t('Default'), rows: 3, rowGap: '6px' },
    { value: 'lg', label: t('Comfortable'), rows: 2, rowGap: '10px' },
    { value: 'xl', label: t('Super Large'), rows: 1, rowGap: '14px' },
  ]
  return (
    <div>
      <SectionTitle
        title={t('Density')}
        showReset={customization.scale !== defaults.scale}
        onReset={() => setScale(defaults.scale)}
      />
      <Radio
        value={customization.scale}
        onValueChange={(v) => setScale(v as ThemeScale)}
        className='grid w-full grid-cols-2 gap-3 sm:grid-cols-4'
        aria-label={t('Select interface density')}
      >
        {scaleOptions.map((option) => (
          <Item
            key={option.value}
            value={option.value}
            className='group flex flex-col items-stretch outline-none'
            aria-label={option.label}
          >
            <div
              className={cn(
                'ring-border/70 bg-card relative h-12 rounded-lg ring-[1px] transition-colors motion-reduce:transition-none',
                'group-data-checked:ring-primary group-data-checked:bg-primary/5',
                'group-focus-visible:ring-2',
                'group-hover:ring-primary/60'
              )}
            >
              <CircleCheck
                className={cn(
                  'fill-primary stroke-primary-foreground absolute top-2 right-2 z-10 size-4',
                  'group-data-unchecked:hidden'
                )}
                aria-hidden='true'
              />
              <ScalePreview rows={option.rows} rowGap={option.rowGap} />
            </div>
            <div className='mt-1.5 text-center text-xs break-words'>
              {option.label}
            </div>
          </Item>
        ))}
      </Radio>
    </div>
  )
}

function SidebarConfig() {
  const { t } = useTranslation()
  const { defaultVariant, variant, setVariant } = useLayout()
  return (
    <div className='max-md:hidden'>
      <SectionTitle
        title={t('Sidebar')}
        showReset={defaultVariant !== variant}
        onReset={() => setVariant(defaultVariant)}
      />
      <Radio
        value={variant}
        onValueChange={setVariant}
        className='grid w-full max-w-md grid-cols-3 gap-4'
        aria-label={t('Select sidebar style')}
        aria-describedby='sidebar-description'
      >
        {[
          { value: 'inset', label: t('Inset'), icon: IconSidebarInset },
          {
            value: 'floating',
            label: t('Floating'),
            icon: IconSidebarFloating,
          },
          { value: 'sidebar', label: t('Sidebar'), icon: IconSidebarSidebar },
        ].map((item) => (
          <RadioGroupItem key={item.value} item={item} />
        ))}
      </Radio>
      <div id='sidebar-description' className='sr-only'>
        {t('Choose between inset, floating, or standard sidebar layout')}
      </div>
    </div>
  )
}

function LayoutConfig() {
  const { t } = useTranslation()
  const { open, setOpen } = useSidebar()
  const { defaultCollapsible, collapsible, setCollapsible } = useLayout()

  const radioState = open ? 'default' : collapsible

  return (
    <div className='max-md:hidden'>
      <SectionTitle
        title={t('Layout')}
        showReset={radioState !== 'default'}
        onReset={() => {
          setOpen(true)
          setCollapsible(defaultCollapsible)
        }}
      />
      <Radio
        value={radioState}
        onValueChange={(v) => {
          if (v === 'default') {
            setOpen(true)
            return
          }
          setOpen(false)
          setCollapsible(v as Collapsible)
        }}
        className='grid w-full max-w-md grid-cols-3 gap-4'
        aria-label={t('Select layout style')}
        aria-describedby='layout-description'
      >
        {[
          { value: 'default', label: t('Default'), icon: IconLayoutDefault },
          { value: 'icon', label: t('Compact'), icon: IconLayoutCompact },
          {
            value: 'offcanvas',
            label: t('Full layout'),
            icon: IconLayoutFull,
          },
        ].map((item) => (
          <RadioGroupItem key={item.value} item={item} />
        ))}
      </Radio>
      <div id='layout-description' className='sr-only'>
        {t(
          'Choose between default expanded, compact icon-only, or full layout mode'
        )}
      </div>
    </div>
  )
}

function ContentLayoutConfig() {
  const { t } = useTranslation()
  const { defaults, customization, setContentLayout } = useThemeCustomization()
  return (
    <div className='max-md:hidden'>
      <SectionTitle
        title={t('Content width')}
        showReset={customization.contentLayout !== defaults.contentLayout}
        onReset={() => setContentLayout(defaults.contentLayout)}
      />
      <Radio
        value={customization.contentLayout}
        onValueChange={(v) => setContentLayout(v as ContentLayout)}
        className='grid w-full grid-cols-2 gap-4'
        aria-label={t('Select content width')}
      >
        {[
          { value: 'full', label: t('Full width') },
          { value: 'centered', label: t('Centered') },
        ].map((option) => (
          <Item
            key={option.value}
            value={option.value}
            className='group flex flex-col items-stretch outline-none'
            aria-label={option.label}
          >
            <div
              className={cn(
                'ring-border/70 bg-card relative h-12 rounded-lg ring-[1px] transition-colors motion-reduce:transition-none',
                'group-data-checked:ring-primary group-data-checked:bg-primary/5',
                'group-focus-visible:ring-2',
                'group-hover:ring-primary/60'
              )}
            >
              <CircleCheck
                className={cn(
                  'fill-primary stroke-primary-foreground absolute top-2 right-2 z-10 size-4',
                  'group-data-unchecked:hidden'
                )}
                aria-hidden='true'
              />
              <ContentLayoutPreview centered={option.value === 'centered'} />
            </div>
            <div className='mt-1.5 text-center text-xs break-words'>
              {option.label}
            </div>
          </Item>
        ))}
      </Radio>
    </div>
  )
}

/**
 * Mini "page" mock used as the visual preview for content-width options.
 * `full` fills horizontally, `centered` clamps the body to a narrow column.
 */
function ContentLayoutPreview(props: { centered: boolean }) {
  return (
    <div aria-hidden='true' className='absolute inset-2 flex flex-col gap-1.5'>
      <span className='bg-foreground/40 block h-1.5 w-full rounded-sm' />
      <div
        className={cn(
          'flex flex-1 flex-col gap-1',
          props.centered ? 'mx-auto w-1/2' : 'w-full'
        )}
      >
        <span className='bg-foreground/60 block h-[2px] w-full rounded-full' />
        <span className='bg-foreground/60 block h-[2px] w-3/4 rounded-full' />
      </div>
    </div>
  )
}

function DirConfig() {
  const { t } = useTranslation()
  const { defaultDir, dir, setDir } = useDirection()
  return (
    <div>
      <SectionTitle
        title={t('Direction')}
        showReset={defaultDir !== dir}
        onReset={() => setDir(defaultDir)}
      />
      <Radio
        value={dir}
        onValueChange={setDir}
        className='grid w-full max-w-md grid-cols-3 gap-4'
        aria-label={t('Select site direction')}
        aria-describedby='direction-description'
      >
        {[
          {
            value: 'ltr',
            label: t('Left to Right'),
            icon: (props: SVGProps<SVGSVGElement>) => (
              <IconDir dir='ltr' {...props} />
            ),
          },
          {
            value: 'rtl',
            label: t('Right to Left'),
            icon: (props: SVGProps<SVGSVGElement>) => (
              <IconDir dir='rtl' {...props} />
            ),
          },
        ].map((item) => (
          <RadioGroupItem key={item.value} item={item} />
        ))}
      </Radio>
      <div id='direction-description' className='sr-only'>
        {t('Choose between left-to-right or right-to-left site direction')}
      </div>
    </div>
  )
}
