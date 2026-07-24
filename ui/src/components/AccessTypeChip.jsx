import { ThemeProvider, createTheme } from '@suid/material'
import Chip from '@suid/material/Chip'

export const makeAccessTypeUserFriendly = (at) => {
	switch (at) {
		case 'A':
			return 'Admin'
		case 'W':
			return 'Edit'
		case 'R':
			return 'View'
		default:
			return at
	}
}

/** Soft tints + dark ink — readable on light and dark themes. */
const accessTypeTheme = createTheme({
	palette: {
		A: {
			main: '#C45C4A',
			light: 'rgba(196, 92, 74, 0.18)',
			dark: '#9E3F30',
			contrastText: '#9E3F30',
		},
		W: {
			main: '#B8862E',
			light: 'rgba(217, 164, 65, 0.22)',
			dark: '#8A6420',
			contrastText: '#8A6420',
		},
		R: {
			main: '#3D4AD6',
			light: 'rgba(91, 108, 255, 0.18)',
			dark: '#2A35A8',
			contrastText: '#2A35A8',
		},
	},
})

/**
 * @typedef {Object} AccessTypeChipProps
 * @property {import('../api').AccessType} at
 */

/**
 * @param {AccessTypeChipProps} props
 */
const AccessTypeChip = (props) => {
	return (
		<span class={`access-chip access-chip--${String(props.at).toLowerCase()}`}>
			{makeAccessTypeUserFriendly(props.at)}
		</span>
	)
}

/**
 * MUI Chip variant kept for places that still need ThemeProvider color.
 * Prefer the CSS `access-chip` span above.
 */
export const AccessTypeMuiChip = (props) => {
	return (
		<ThemeProvider theme={accessTypeTheme}>
			<Chip
				label={makeAccessTypeUserFriendly(props.at)}
				color={props.at}
				size="small"
				sx={{
					fontWeight: 700,
					bgcolor: (t) => t.palette[props.at]?.light,
					color: (t) => t.palette[props.at]?.contrastText,
					border: (t) => `1px solid ${t.palette[props.at]?.main}`,
				}}
			/>
		</ThemeProvider>
	)
}

export default AccessTypeChip
