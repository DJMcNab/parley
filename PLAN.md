# Apple System Emoji on Web

We want to make an example which proves that it's possible for an application using Parley and Vello CPU to draw text using Apple's system emoji, whilst rendering on the web.
We cannot download the emoji font from an online source, so we need to use the version on the user's system using web APIs.
DO NOT USE A SOLUTION WHICH EMBEDS THE APPLE EMOJI FONT, OR REQUIRES IT TO BE SERVED.
This solution needs to work across all major browsers (Chrome, Safari, Firefox), as well as Safari on iOS and iPadOS.
However, validating this on iOS and iPads is out of scope for this task.

I've downloaded and installed Chrome for Testing 150.0.7871.46, alongside it's applicable ChromeDriver.
These are at:
/Users/djmcnab/chromedriver/mac_arm-150.0.7871.46/chromedriver-mac-arm64/chromedriver
and
/Users/djmcnab/chrome/mac_arm-150.0.7871.46/chrome-mac-arm64/Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing

For this testing, we should use the Arimo font downloaded in this repository.

## Phase One

We believe that the approach which will make this work is to render the emoji in a 2d canvas context, reading the metrics from there.
Please make a Rust app which draws the system emoji into a 2d canvas, and reads as many metrics as feasible about it.
This app should be in `examples/apple_emoji/phase_one`.
In particular, we need the "advance width", and we also need to download the Emoji content into the Rust code, ideally ensuring that no clipping has occurred.
Prove that the Rust code has the emoji pixels before continuing, such as by manually drawing it to a second canvas.

Do not commit this work.
Write the relevant plans into `examples/apple_moji/phase_one/plans`.
You should not make changes to the Parley Library code to achieve this.
Stop here if this isn't feasible or you can't validate it.

## Phase Two

Please make a Rust app which uses Parley and Vello CPU to draw some example text. We can them replace each Emoji with an inline box.
This app should be in `examples/apple_emoji/phase_two`, and you should not edit `phase_one` as part of this task; when you need code from it, copy that code.
At drawing time, these inline boxes should be replaced with system emoji, achieved using step 1.
The emoji should be made to take as close to the same advance width that the Emoji would take if using the Apple System font.
We also need the Emoji to be aligned to the correct baseline.
To validate this, take screenshots of the same text using the same font, but rendered by the browser.

Do not commit this work.
Write the relevant plans into `examples/apple_moji/phase_two/plans`.
You should not make changes to the Parley Library code to achieve this.
Stop here if this isn't feasible or you can't validate it.

## Phase Three

For this phase, I have downloaded NotoColorEmoji-Regular.ttf.
I want this work to happen in `examples/apple_emoji/phase_three`, and you should not edit `phase_one` or `phase_two` as part of this task; when you need code from it, copy that code.

I want to make these rendered Emoji be rendered as if they were metrically compatible with Noto Color Emoji.
We would achieve this by scaling the Emoji linearly to match the advance width of their Noto Color Emoji equivalent.
We need to achieve this by downloading as little extra data from Noto Color Emoji as possible.
In particular, we cannot just download the entire font for this.

The approach we should use first is:

- Creating a data file containing the advance width of every glyph in Noto Color Emoji.
- Using these widths to draw the inline boxes from phase two.
- We should validate that this gives the same total advance width as the same text drawn with Noto Color Emoji, but with Apple System Emoji.
  Don't worry about vertical alignment/the baselines lining up.

Do not commit this work.
Write the relevant plans into `examples/apple_moji/phase_three/plans`.
You should not make changes to the Parley Library code to achieve this.
Stop here if this isn't feasible or you can't validate it.

## Phase Four

We want to also make it match the baselines. This should happen in `examples/apple_emoji/phase_four`, with the same rules as before.
The proposed approach is:

- to make a small (less than 150KiB) fake font which has the metrics of Noto Color Emoji, but none of the glyph data.
- Draw this in Parley, but replacing each Fake Noto Emoji with the equivalent scaled system Emoji.
- Validate that this does render the same metrics as the full Noto Color Emoji before continuing.
- You can make changes to Parley to enable this. However, you should try and achieve this with as few changes to Parley as possible.
  If you need to, think through at least two possible API changes, and make plans for both. Choose the one you think is best, but makes sure that both plans are available
  (in `examples/apple_moji/phase_four/plans`)

Do not commit this work.
Write the relevant plans into `examples/apple_moji/phase_four/plans`.
