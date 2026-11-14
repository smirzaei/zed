use std::borrow::Cow;

use anyhow::{Context as _, Result, bail, ensure};

use crate::{
    AtlasKey, AtlasTextureKind, Bounds, DevicePixels, FontId, Hsla, MonochromeSprite, Pixels,
    Point, PolychromeSprite, Size, SubpixelSprite, TransformationMatrix, Window,
};

/// Identifies a bitmap that an app rasterized itself, in a window's sprite atlas.
///
/// The `u64` is the app's choice. Two bitmaps that differ in any way, such as in the scale factor
/// they were rasterized at, need different ids. Ids of the three kinds don't clash.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AppSpriteKey {
    /// A mask with one byte of coverage per pixel. It is painted in a color, as a monochrome
    /// sprite, so tests read it through `Window::painted_monochrome_sprites`.
    Mask(u64),
    /// A color bitmap with four bytes per pixel in blue, green, red, alpha order, where the color
    /// channels are not premultiplied by alpha.
    Color(u64),
    /// A mask with four bytes per pixel: the coverage of the pixel's red, green, and blue
    /// subpixels in that order, then a byte that is ignored. It is painted in a color, as GPUI
    /// paints its own subpixel text, so tests read it through `Window::painted_subpixel_sprites`.
    /// An app paints it only where [`Window::paints_subpixel_text`] says the window does, and a
    /// window whose renderer has no subpixel masks refuses it.
    Subpixel(u64),
}

impl AppSpriteKey {
    pub(crate) fn texture_kind(self) -> AtlasTextureKind {
        match self {
            Self::Mask(_) => AtlasTextureKind::Monochrome,
            Self::Color(_) => AtlasTextureKind::Polychrome,
            Self::Subpixel(_) => AtlasTextureKind::Subpixel,
        }
    }

    fn check_bitmap(self, size: Size<DevicePixels>, bytes: &[u8]) -> Result<()> {
        let bytes_per_pixel = match self {
            Self::Mask(_) => 1,
            Self::Color(_) | Self::Subpixel(_) => 4,
        };
        ensure!(
            size.width.0 > 0 && size.height.0 > 0,
            "the bitmap of {self:?} is empty: {size:?}"
        );
        let expected_len = (size.width.0 as usize)
            .checked_mul(size.height.0 as usize)
            .and_then(|pixels| pixels.checked_mul(bytes_per_pixel));
        ensure!(
            expected_len == Some(bytes.len()),
            "the bitmap of {self:?} is {size:?} but has {} bytes",
            bytes.len()
        );
        Ok(())
    }

    /// Reorders the bytes of a checked bitmap into the order the platform's atlas holds them in.
    /// Direct3D's atlas holds subpixel masks as RGBA. Every other renderer's holds them as BGRA,
    /// as it holds color bitmaps.
    fn reorder_for_atlas(self, bytes: &mut [u8]) {
        if !matches!(self, Self::Subpixel(_)) || cfg!(target_os = "windows") {
            return;
        }
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
    }
}

impl Window {
    /// Returns whether the window paints text in `font_id` at `font_size` with subpixel masks, as
    /// it does its own glyphs.
    ///
    /// An app that rasterizes glyphs itself asks this to match the text around them: it paints
    /// [`AppSpriteKey::Subpixel`] bitmaps when this is true and [`AppSpriteKey::Mask`] bitmaps
    /// otherwise. For glyphs in a font that GPUI's text system doesn't know, it passes the font
    /// of the text around them. The answer changes with the window's background and the text
    /// rendering mode.
    pub fn paints_subpixel_text(&self, font_id: FontId, font_size: Pixels) -> bool {
        self.should_use_subpixel_rendering(font_id, font_size)
    }

    /// For testing: sets whether the test window reports subpixel support, and schedules a redraw.
    /// Panics unless this is a test window.
    #[cfg(any(test, feature = "test-support"))]
    pub fn set_subpixel_rendering_supported(&mut self, supported: bool) {
        let test_window = self
            .platform_window
            .as_test()
            .expect("only a test window's subpixel support can be set");
        test_window.0.lock().subpixel_rendering_supported = supported;
        self.refresh();
    }

    /// Paints a bitmap, which the app rasterized itself, into the scene for the next frame at the
    /// current z-index.
    ///
    /// `origin` is the bitmap's top-left corner in the window's device pixels, so the app decides
    /// how it snaps to the pixel grid. The atlas keeps the bitmap under `key`, and `rasterize`
    /// runs only when it isn't there. It returns the bitmap's size, which can't be empty, and its
    /// pixels row by row from the top, with no padding, in the layout `key` documents.
    ///
    /// A mask is painted in `color`. A color bitmap keeps its own colors and ignores it. Element
    /// opacity and the content mask apply as they do to glyphs. When `rasterize` fails or returns
    /// a bitmap that doesn't match its size, the error is returned and nothing is stored or
    /// painted. A subpixel mask is refused likewise in a window whose renderer can't paint them,
    /// such as on macOS.
    ///
    /// This method should only be called as part of the paint phase of element drawing.
    pub fn paint_app_sprite(
        &mut self,
        origin: Point<DevicePixels>,
        key: AppSpriteKey,
        color: Hsla,
        rasterize: impl FnOnce() -> Result<(Size<DevicePixels>, Vec<u8>)>,
    ) -> Result<()> {
        self.invalidator.debug_assert_paint();

        if matches!(key, AppSpriteKey::Subpixel(_))
            && !self.platform_window.is_subpixel_rendering_supported()
        {
            bail!("the window's renderer can't paint {key:?}");
        }

        let mut rasterize = Some(rasterize);
        let tile = self
            .sprite_atlas
            .get_or_insert_with(AtlasKey::App(key), &mut || {
                let rasterize = rasterize
                    .take()
                    .context("the atlas built an app sprite twice")?;
                let (size, mut bytes) = rasterize()?;
                key.check_bitmap(size, &bytes)?;
                key.reorder_for_atlas(&mut bytes);
                Ok(Some((size, Cow::Owned(bytes))))
            })?
            .expect("Callback above only errors or returns Some");
        let bounds = Bounds {
            origin: origin.map(Into::into),
            size: tile.bounds.size.map(Into::into),
        };
        let content_mask = self.snapped_content_mask();
        let opacity = self.element_opacity();

        match key {
            AppSpriteKey::Mask(_) => self.next_frame.scene.insert_primitive(MonochromeSprite {
                order: 0,
                pad: 0,
                bounds,
                content_mask,
                color: color.opacity(opacity),
                tile,
                transformation: TransformationMatrix::unit(),
            }),
            AppSpriteKey::Color(_) => self.next_frame.scene.insert_primitive(PolychromeSprite {
                order: 0,
                pad: 0,
                grayscale: false.into(),
                bounds,
                corner_radii: Default::default(),
                content_mask,
                tile,
                opacity,
            }),
            AppSpriteKey::Subpixel(_) => self.next_frame.scene.insert_primitive(SubpixelSprite {
                order: 0,
                pad: 0,
                bounds,
                content_mask,
                color: color.opacity(opacity),
                tile,
                transformation: TransformationMatrix::unit(),
            }),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    use anyhow::{Context as _, Result};

    use crate::{
        AnyWindowHandle, AppContext as _, AppSpriteKey, AtlasKey, AtlasTextureKind, Bounds,
        ContentMask, Context, DevicePixels, FontId, Hsla, IntoElement, Point, Render, ScaledPixels,
        Size, Styled, TestAppContext, TextRenderingMode, Window, canvas, hsla, point, px, size,
    };

    const RED: Hsla = hsla(0., 1., 0.5, 1.);

    struct Painter<F>(F);

    impl<F: Fn(&mut Window) + Clone + 'static> Render for Painter<F> {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let paint = self.0.clone();
            canvas(|_, _, _| {}, move |_, _, window, _| paint(window)).size_full()
        }
    }

    fn paint_window(
        cx: &mut TestAppContext,
        paint: impl Fn(&mut Window) + Clone + 'static,
    ) -> AnyWindowHandle {
        cx.add_window(|_, _| Painter(paint)).into()
    }

    fn redraw(cx: &mut TestAppContext, window: AnyWindowHandle) {
        cx.update_window(window, |_, window, cx| window.draw(cx).clear(cx))
            .unwrap();
    }

    fn set_subpixel_support(cx: &mut TestAppContext, window: AnyWindowHandle, supported: bool) {
        cx.update_window(window, |_, window, _| {
            window.set_subpixel_rendering_supported(supported)
        })
        .unwrap();
    }

    /// A window that supports subpixel masks and has painted once with them.
    fn subpixel_window(
        cx: &mut TestAppContext,
        paint: impl Fn(&mut Window) + Clone + 'static,
    ) -> AnyWindowHandle {
        let window = paint_window(cx, move |window| {
            if window.platform_window.is_subpixel_rendering_supported() {
                paint(window);
            }
        });
        set_subpixel_support(cx, window, true);
        window
    }

    fn bitmap(width: i32, height: i32, bytes_per_pixel: usize) -> (Size<DevicePixels>, Vec<u8>) {
        let bytes = vec![0xff; width as usize * height as usize * bytes_per_pixel];
        (size(DevicePixels(width), DevicePixels(height)), bytes)
    }

    fn device_origin(x: i32, y: i32) -> Point<DevicePixels> {
        point(DevicePixels(x), DevicePixels(y))
    }

    fn scaled_bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<ScaledPixels> {
        Bounds::new(
            point(ScaledPixels(x), ScaledPixels(y)),
            size(ScaledPixels(width), ScaledPixels(height)),
        )
    }

    #[gpui::test]
    fn test_mask_paints_in_color_at_device_origin(cx: &mut TestAppContext) {
        let window = paint_window(cx, |window| {
            let (size, bytes) = bitmap(2, 3, 1);
            window
                .paint_app_sprite(device_origin(5, 7), AppSpriteKey::Mask(1), RED, move || {
                    Ok((size, bytes))
                })
                .unwrap();
        });

        cx.update_window(window, |_, window, _| {
            let sprites = window.painted_monochrome_sprites();
            assert_eq!(sprites.len(), 1);
            assert_eq!(sprites[0].bounds, scaled_bounds(5., 7., 2., 3.));
            assert_eq!(sprites[0].color, RED);
            assert_eq!(
                sprites[0].tile.texture_id.kind,
                AtlasTextureKind::Monochrome
            );
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_color_sprite_paints_untinted_and_apart_from_a_mask_with_the_same_id(
        cx: &mut TestAppContext,
    ) {
        let window = paint_window(cx, |window| {
            window
                .paint_app_sprite(device_origin(0, 0), AppSpriteKey::Mask(1), RED, || {
                    Ok(bitmap(2, 2, 1))
                })
                .unwrap();
            window
                .paint_app_sprite(device_origin(10, 0), AppSpriteKey::Color(1), RED, || {
                    Ok(bitmap(3, 2, 4))
                })
                .unwrap();
        });

        cx.update_window(window, |_, window, _| {
            let masks = window.painted_monochrome_sprites();
            let colors = &window.rendered_frame.scene.polychrome_sprites;
            assert_eq!(masks.len(), 1);
            assert_eq!(colors.len(), 1);
            assert_eq!(masks[0].bounds, scaled_bounds(0., 0., 2., 2.));
            assert_eq!(colors[0].bounds, scaled_bounds(10., 0., 3., 2.));
            assert_eq!(colors[0].tile.texture_id.kind, AtlasTextureKind::Polychrome);
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_sprite_rasterizes_once_across_redraws(cx: &mut TestAppContext) {
        let painted = Rc::new(Cell::new(0));
        let rasterized = Rc::new(Cell::new(0));
        let window = paint_window(cx, {
            let painted = painted.clone();
            let rasterized = rasterized.clone();
            move |window| {
                painted.set(painted.get() + 1);
                window
                    .paint_app_sprite(device_origin(0, 0), AppSpriteKey::Mask(1), RED, || {
                        rasterized.set(rasterized.get() + 1);
                        Ok(bitmap(2, 2, 1))
                    })
                    .unwrap();
            }
        });
        redraw(cx, window);
        assert_eq!(painted.get(), 2);
        assert_eq!(rasterized.get(), 1);

        cx.update_window(window, |_, window, _| {
            assert_eq!(window.painted_monochrome_sprites().len(), 1);
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_opacity_and_content_mask_apply(cx: &mut TestAppContext) {
        let window = paint_window(cx, |window| {
            let mask = ContentMask {
                bounds: Bounds::new(point(px(0.), px(0.)), size(px(4.), px(4.))),
            };
            window.with_content_mask(Some(mask), |window| {
                window.with_element_opacity(Some(0.5), |window| {
                    window
                        .paint_app_sprite(device_origin(2, 2), AppSpriteKey::Mask(1), RED, || {
                            Ok(bitmap(2, 2, 1))
                        })
                        .unwrap();
                    window
                        .paint_app_sprite(device_origin(2, 2), AppSpriteKey::Color(1), RED, || {
                            Ok(bitmap(2, 2, 4))
                        })
                        .unwrap();
                    window
                        .paint_app_sprite(device_origin(50, 50), AppSpriteKey::Mask(1), RED, || {
                            Ok(bitmap(2, 2, 1))
                        })
                        .unwrap();
                });
            });
        });

        cx.update_window(window, |_, window, _| {
            let clip = Bounds::new(point(px(0.), px(0.)), size(px(4.), px(4.)))
                .scale(window.scale_factor());
            let masks = window.painted_monochrome_sprites();
            let colors = &window.rendered_frame.scene.polychrome_sprites;
            assert_eq!(masks.len(), 1, "the sprite outside the mask is culled");
            assert_eq!(colors.len(), 1);
            assert_eq!(masks[0].content_mask.bounds, clip);
            assert_eq!(colors[0].content_mask.bounds, clip);
            assert_eq!(masks[0].color, RED.opacity(0.5));
            assert_eq!(colors[0].opacity, 0.5);
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_rejected_bitmaps_are_neither_stored_nor_painted(cx: &mut TestAppContext) {
        let next_bitmap = Rc::new(RefCell::new(None));
        let outcomes: Rc<RefCell<Vec<Result<()>>>> = Rc::default();
        let window = paint_window(cx, {
            let next_bitmap = next_bitmap.clone();
            let outcomes = outcomes.clone();
            move |window| {
                let outcome = window.paint_app_sprite(
                    device_origin(0, 0),
                    AppSpriteKey::Mask(1),
                    RED,
                    || {
                        next_bitmap
                            .borrow_mut()
                            .take()
                            .context("failed to rasterize")
                    },
                );
                outcomes.borrow_mut().push(outcome);
            }
        });

        let key = AtlasKey::App(AppSpriteKey::Mask(1));
        let rejected = [
            None,
            Some(bitmap(0, 0, 1)),
            Some(bitmap(2, 2, 4)),
            Some((size(DevicePixels(2), DevicePixels(2)), vec![0xff; 3])),
        ];
        for next in rejected {
            *next_bitmap.borrow_mut() = next;
            redraw(cx, window);
            assert!(outcomes.borrow_mut().pop().unwrap().is_err());
            cx.update_window(window, |_, window, _| {
                assert!(window.painted_monochrome_sprites().is_empty());
                assert!(!window.sprite_atlas.contains(&key));
            })
            .unwrap();
        }

        *next_bitmap.borrow_mut() = Some(bitmap(2, 2, 1));
        redraw(cx, window);
        assert!(outcomes.borrow_mut().pop().unwrap().is_ok());
        cx.update_window(window, |_, window, _| {
            assert_eq!(window.painted_monochrome_sprites().len(), 1);
            assert!(window.sprite_atlas.contains(&key));
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_subpixel_sprite_paints_in_color_apart_from_a_mask_with_the_same_id(
        cx: &mut TestAppContext,
    ) {
        let window = subpixel_window(cx, |window| {
            let mask = ContentMask {
                bounds: Bounds::new(point(px(0.), px(0.)), size(px(4.), px(4.))),
            };
            window.with_content_mask(Some(mask), |window| {
                window.with_element_opacity(Some(0.5), |window| {
                    window
                        .paint_app_sprite(
                            device_origin(2, 2),
                            AppSpriteKey::Subpixel(1),
                            RED,
                            || Ok(bitmap(2, 2, 4)),
                        )
                        .unwrap();
                    window
                        .paint_app_sprite(device_origin(2, 2), AppSpriteKey::Mask(1), RED, || {
                            Ok(bitmap(2, 2, 1))
                        })
                        .unwrap();
                    window
                        .paint_app_sprite(
                            device_origin(50, 50),
                            AppSpriteKey::Subpixel(1),
                            RED,
                            || Ok(bitmap(2, 2, 4)),
                        )
                        .unwrap();
                });
            });
        });

        cx.update_window(window, |_, window, _| {
            let clip = Bounds::new(point(px(0.), px(0.)), size(px(4.), px(4.)))
                .scale(window.scale_factor());
            let subpixel = window.painted_subpixel_sprites();
            let masks = window.painted_monochrome_sprites();
            assert_eq!(subpixel.len(), 1, "the sprite outside the mask is culled");
            assert_eq!(masks.len(), 1);
            assert_eq!(subpixel[0].bounds, scaled_bounds(2., 2., 2., 2.));
            assert_eq!(subpixel[0].content_mask.bounds, clip);
            assert_eq!(subpixel[0].color, RED.opacity(0.5));
            assert_eq!(subpixel[0].tile.texture_id.kind, AtlasTextureKind::Subpixel);
            assert_eq!(masks[0].tile.texture_id.kind, AtlasTextureKind::Monochrome);
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_subpixel_sprite_is_refused_where_the_renderer_has_none(cx: &mut TestAppContext) {
        let outcomes: Rc<RefCell<Vec<Result<()>>>> = Rc::default();
        let rasterized = Rc::new(Cell::new(0));
        let window = paint_window(cx, {
            let outcomes = outcomes.clone();
            let rasterized = rasterized.clone();
            move |window| {
                let outcome = window.paint_app_sprite(
                    device_origin(0, 0),
                    AppSpriteKey::Subpixel(1),
                    RED,
                    || {
                        rasterized.set(rasterized.get() + 1);
                        Ok(bitmap(2, 2, 4))
                    },
                );
                outcomes.borrow_mut().push(outcome);
            }
        });

        let key = AtlasKey::App(AppSpriteKey::Subpixel(1));
        assert!(outcomes.borrow_mut().pop().unwrap().is_err());
        assert_eq!(rasterized.get(), 0);
        cx.update_window(window, |_, window, _| {
            assert!(window.painted_subpixel_sprites().is_empty());
            assert!(!window.sprite_atlas.contains(&key));
        })
        .unwrap();

        set_subpixel_support(cx, window, true);
        assert!(outcomes.borrow_mut().pop().unwrap().is_ok());
        assert_eq!(rasterized.get(), 1);
        cx.update_window(window, |_, window, _| {
            assert_eq!(window.painted_subpixel_sprites().len(), 1);
            assert!(window.sprite_atlas.contains(&key));
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_subpixel_sprite_needs_four_bytes_per_pixel(cx: &mut TestAppContext) {
        let outcomes: Rc<RefCell<Vec<Result<()>>>> = Rc::default();
        let window = subpixel_window(cx, {
            let outcomes = outcomes.clone();
            move |window| {
                let outcome = window.paint_app_sprite(
                    device_origin(0, 0),
                    AppSpriteKey::Subpixel(1),
                    RED,
                    || Ok(bitmap(2, 2, 1)),
                );
                outcomes.borrow_mut().push(outcome);
            }
        });

        assert!(outcomes.borrow_mut().pop().unwrap().is_err());
        cx.update_window(window, |_, window, _| {
            assert!(window.painted_subpixel_sprites().is_empty());
            assert!(
                !window
                    .sprite_atlas
                    .contains(&AtlasKey::App(AppSpriteKey::Subpixel(1)))
            );
        })
        .unwrap();
    }

    #[gpui::test]
    fn test_paints_subpixel_text_needs_support_and_the_subpixel_mode(cx: &mut TestAppContext) {
        let window = paint_window(cx, |_| {});
        let paints_subpixel_text = |cx: &mut TestAppContext| {
            cx.update_window(window, |_, window, _| {
                window.paints_subpixel_text(FontId(0), px(14.))
            })
            .unwrap()
        };

        cx.update(|cx| cx.set_text_rendering_mode(TextRenderingMode::Subpixel));
        assert!(!paints_subpixel_text(cx), "the window has no support");

        set_subpixel_support(cx, window, true);
        assert!(paints_subpixel_text(cx));

        cx.update(|cx| cx.set_text_rendering_mode(TextRenderingMode::Grayscale));
        assert!(!paints_subpixel_text(cx));
    }

    #[test]
    fn test_subpixel_bytes_reach_the_atlas_in_its_order() {
        let (red, green, blue, ignored) = (1, 2, 3, 4);
        let mut subpixel = [red, green, blue, ignored, 5, 6, 7, 8];
        let mut color = subpixel;
        let mut mask = subpixel;
        AppSpriteKey::Subpixel(1).reorder_for_atlas(&mut subpixel);
        AppSpriteKey::Color(1).reorder_for_atlas(&mut color);
        AppSpriteKey::Mask(1).reorder_for_atlas(&mut mask);

        let expected = if cfg!(target_os = "windows") {
            [red, green, blue, ignored, 5, 6, 7, 8]
        } else {
            [blue, green, red, ignored, 7, 6, 5, 8]
        };
        assert_eq!(subpixel, expected);
        assert_eq!(color, [red, green, blue, ignored, 5, 6, 7, 8]);
        assert_eq!(mask, [red, green, blue, ignored, 5, 6, 7, 8]);
    }
}
