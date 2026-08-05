import { Show } from 'solid-js'
import Box from '@suid/material/Box'

import { resolveMaterialIconUrl } from '../common/materialFileIcons'
import FluentIcon from './FluentIcon'

const SIZE = 48

/**
 * @typedef {Object} FileTypeIconProps
 * @property {string} name
 * @property {boolean} [isFile]
 * @property {boolean} [open] folder-open glyph when true
 * @property {boolean} [storage] Fluent cloud glyph for storage cards
 * @property {string} [thumbUrl] when set, shows thumbnail instead of glyph
 * @property {number} [size]
 * @property {() => void} [onThumbError] Called when `thumbUrl` fails to decode
 *   (e.g. a revoked blob: URL) so the caller clears it and this falls back to
 *   the material glyph instead of staying on the broken-image "?" forever.
 */

/**
 * File-type glyph from material-icon-theme (no plate — full icon size).
 * Chrome UI icons stay on Fluent; storage cards use Fluent cloud.
 *
 * @param {FileTypeIconProps} props
 */
const FileTypeIcon = (props) => {
	const size = () => props.size || SIZE
	const src = () =>
		resolveMaterialIconUrl(props.name, {
			isFile: props.isFile !== false,
			open: !!props.open,
		})

	return (
		<Show
			when={props.thumbUrl}
			fallback={
				<Show
					when={props.storage}
					fallback={
						<span
							class="file-type-icon file-type-icon--bare"
							style={{ width: `${size()}px`, height: `${size()}px` }}
							aria-hidden="true"
						>
							<img
								src={src()}
								alt=""
								draggable={false}
								class="file-type-icon__glyph"
							/>
						</span>
					}
				>
					<span
						class="file-type-icon file-type-icon--bare file-type-icon--storage"
						style={{ width: `${size()}px`, height: `${size()}px` }}
						aria-hidden="true"
					>
						<FluentIcon name="cloudFilled" size={Math.round(size() * 0.78)} />
					</span>
				</Show>
			}
		>
			<Box
				component="img"
				src={props.thumbUrl}
				alt=""
				class="file-type-icon file-type-icon--thumb"
				sx={{
					width: size(),
					height: size(),
					objectFit: 'cover',
				}}
				onError={() => props.onThumbError?.()}
			/>
		</Show>
	)
}

export default FileTypeIcon
