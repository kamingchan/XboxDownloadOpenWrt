'use strict';
'require view';
'require form';
'require uci';
'require xboxdownload as xb';

function choices(option, values) {
	Object.keys(values).forEach(function(k) { option.value(k, values[k]); });
	return option;
}
function setting(option, datatype, def) {
	if (datatype) option.datatype = datatype;
	if (def != null) { option.default = def; option.rmempty = false; }
	option.retain = true;
	return option;
}
function webURL(id, value) {
	if (!value) return true;
	try { var u = new URL(value); if (/^https?:$/.test(u.protocol) && u.hostname) return true; } catch (e) {}
	return _('Enter an HTTP or HTTPS URL.');
}
function upstreamLabel(candidate) {
	var sources = { dnsmasq_upstream: _('dnsmasq upstream'), local_service: _('Local DNS service'),
		wan_dns: _('WAN DNS'), public: _('Public DNS') };
	var services = { SmartDNS: _('SmartDNS'), AliDNS: _('AliDNS'), DNSPod: _('DNSPod'),
		Cloudflare: _('Cloudflare'), Google: _('Google') };
	var source = sources[candidate.source], service = services[candidate.service];
	var label = service && candidate.source !== 'public' ? _('%s (%s)').format(service, source) : service || source;
	return label ? _('%s — %s').format(candidate.address, label) : candidate.address;
}
function copy(area) {
	if (navigator.clipboard && window.isSecureContext)
		return navigator.clipboard.writeText(area.value).then(function() { xb.notify(_('Copied.')); });
	area.focus(); area.select();
	if (document.execCommand('copy')) xb.notify(_('Copied.'));
	else xb.notify(_('Select and copy the rules manually.'));
}

return view.extend({
	load: function() {
		return Promise.all([ uci.load('xboxdownload'), xb.suggest_upstream_dns().catch(function() { return { candidates: [] }; }) ]);
	},
	render: function(data) {
		var m = new form.Map('xboxdownload', _('Xbox Download'));
		var s = m.section(form.NamedSection, 'main', 'main');
		s.tab('network', _('Basic settings and network'));
		s.tab('groups', _('Group policies'));
		s.tab('schedule', _('Automatic tests and IP sync'));
		s.tab('ipv6', _('IPv6 fallback'));
		function main(type, key, label, datatype, def, description) {
			return setting(s.taboption('network', type, key, label, description), datatype, def);
		}
		var on = main(form.Flag, 'enabled', _('Enable service'), null, '0');
		choices(main(form.ListValue, 'dns_mode', _('DNS integration'), null, 'dnsmasq'),
			{ dnsmasq: _('Automatic dnsmasq forwarding'), manual: _('Manual forwarding') });
		var bind = main(form.Value, 'dns_bind', _('DNS bind address'), 'ip4addr(1)', '127.0.0.1');
		var port = main(form.Value, 'dns_port', _('DNS port'), 'and(port,min(1))', '5353');
		var upstream = main(form.Value, 'upstream_dns', _('Fallback DNS server'), 'ipaddrport(1)', '223.5.5.5:53',
			_('Use an independent upstream resolver. A loop back to the router DNS listener will prevent startup.'));
		function upstreamChoices(candidates) {
			upstream.keylist = []; upstream.vallist = [];
			candidates.forEach(function(c) { upstream.value(c.address, upstreamLabel(c)); });
		}
		upstreamChoices(data[1].candidates);
		var detect = main(form.Button, '_detect_upstream', '');
		detect.inputtitle = _('Detect upstream DNS');
		detect.onclick = function() {
			return xb.suggest_upstream_dns().then(function(result) {
				var detected = result.candidates.filter(function(c) { return c.source !== 'public'; });
				var selected = detected.length ? detected[0].address : upstream.formvalue('main');
				upstreamChoices(result.candidates);
				var widget = m.findElement('id', upstream.cbid('main'));
				widget.replaceWith(upstream.renderWidget('main', 0, selected));
				if (detected.length)
					xb.notify(_('Detected upstream DNS servers: %s. Selected %s. Save and apply to use it.').format(
						detected.map(function(c) { return c.address; }).join(', '), selected));
				else if (result.candidates.length)
					xb.notify(_('No router or WAN upstream DNS found. Choose a public DNS server from the list or enter a custom address.'));
				else
					xb.notify(_('No upstream DNS candidates found. Enter a custom address.'));
			}).catch(function(e) { xb.notify(e.message, 'error'); });
		};
		main(form.Value, 'dns_ttl', _('DNS TTL (seconds)'), 'uinteger', '60');
		var blocks = {}, groupSections = {};
		var snippets = main(form.DummyValue, '_manual', _('Manual DNS rules'));
		snippets.depends('dns_mode', 'manual');
		snippets.renderWidget = function() {
			return E('div', { 'class': 'xbox-rules' }, [ E('p', {}, _('Copy the rules for the DNS service on this router.')) ].concat(
				[ 'SmartDNS', 'AdGuardHome', 'dnsmasq' ].map(function(name) {
					var area = E('textarea', { readonly: true, rows: 5, spellcheck: false, 'aria-label': name });
					blocks[name] = area;
					return E('details', {}, [ E('summary', {}, name), area, xb.button(_('Copy rules'), function() { return copy(area); }) ]);
				})));
		};
		var mode = choices(main(form.ListValue, 'http_mode', _('HTTP interception'), null, 'alias_ip'),
			{ alias_ip: _('LAN alias IP'), custom_port: _('Custom listener') });
		var lan = main(form.Value, 'lan_device', _('LAN device'), null, 'br-lan');
		lan.validate = function(id, v) { return /^[a-zA-Z0-9_.][a-zA-Z0-9_.-]*$/.test(v) || _('Enter a LAN device name.'); };
		var alias = main(form.Value, 'alias_ip', _('LAN alias IPv4 address'), 'ip4addr(1)', null,
			_('Reserve this address outside your DHCP pool. A sleeping device may not answer a probe.'));
		alias.depends('http_mode', 'alias_ip');
		on.validate = function(id, v) {
			return v !== '1' || mode.formvalue(id) !== 'alias_ip' || !!alias.formvalue(id) || _('Choose a LAN alias address before enabling the service.');
		};
		var suggest = main(form.Button, '_suggest', _('Find an unused IP'));
		suggest.inputtitle = _('Suggest LAN address'); suggest.depends('http_mode', 'alias_ip');
		suggest.onclick = function() {
			return xb.suggest_alias_ip(lan.formvalue('main')).then(function(r) {
				if (!r.suggested_ip || r.in_use) throw new Error(_('No available address found in the probed range.'));
				alias.getUIElement('main').setValue(r.suggested_ip);
				xb.notify(_('Suggested %s on %s. Save and apply to use it.').format(r.suggested_ip, r.cidr));
			}).catch(function(e) { xb.notify(e.message, 'error'); });
		};
		main(form.Value, 'http_port', _('HTTP listener port'), 'and(port,min(1))', '18080');
		main(form.Value, 'http_bind', _('HTTP bind address'), 'ip4addr(1)', '0.0.0.0').depends('http_mode', 'custom_port');
		main(form.Value, 'redirect_ip', _('Redirect target IPv4'), 'ip4addr(1)', null,
			_('Defaults to the LAN address. Clients must reach the redirect listener on port 80.')).depends('http_mode', 'custom_port');
		main(form.Value, 'log_capacity', _('Redirect log capacity'), 'uinteger', '200');
		Object.keys(xb.groups).forEach(function(id) {
			var group = s.taboption('groups', form.SectionValue, '_group_' + id, form.NamedSection, id, 'group', xb.groups[id]).subsection;
			groupSections[id] = group;
			setting(group.option(form.Flag, 'enabled', _('Enable group')), null, id === 'akamai' ? '0' : '1');
			choices(setting(group.option(form.ListValue, 'ip_family', _('Address family')), null, 'ipv4'), xb.families);
			setting(group.option(form.Value, 'selected_ipv4', _('Selected IPv4')), 'ip4addr(1)');
			setting(group.option(form.Value, 'selected_ipv6', _('Selected IPv6')), 'ip6addr(1)');
			group.option(form.Value, 'filter_keyword', _('Filter keyword'));
			group.option(form.Value, 'custom_test_url', _('Custom test URL')).validate = webURL;
			if (id !== 'akamai')
				setting(group.option(form.Flag, 'use_akamai_fallback', _('Use Akamai fallback'),
					_('When this group has no selected address, use a compatible address from the enabled Akamai group.')), null, '0');
		});
		var speed = s.taboption('schedule', form.SectionValue, '_speed', form.NamedSection, 'speedtest', 'speedtest').subsection;
		function test(type, key, label, datatype, def, help) {
			return setting(speed.option(type, key, label, help), datatype, def);
		}
		test(form.Flag, 'auto_enabled', _('Enable automatic tests'), null, '1');
		test(form.Value, 'interval_hours', _('Test interval (hours)'), 'and(uinteger,min(1))', '12');
		test(form.Value, 'prefer_hour', _('Preferred hour (0–23, or -1 for any hour)'), 'and(integer,range(-1,23))', '4');
		test(form.Value, 'defer_active_minutes', _('Defer after a redirect (minutes)'), 'uinteger', '15');
		test(form.Value, 'switch_threshold_pct', _('Required speed improvement (%)'), 'ufloat', '20');
		test(form.Value, 'min_speed_mbps', _('Minimum current speed (MB/s)'), 'ufloat', '5.0',
			_('A failed or slower-than-minimum node can be replaced without the improvement threshold. Failed tests keep the current IP.'));
		test(form.Value, 'prefilter_concurrency', _('Concurrent TCP probes'), 'and(uinteger,min(1))', '24');
		test(form.Value, 'prefilter_top_n', _('Candidates for download testing'), 'and(uinteger,min(1))', '10');
		test(form.Value, 'download_concurrency', _('Concurrent downloads'), 'and(uinteger,min(1))', '4');
		test(form.Value, 'download_range_mb', _('Download range per node (MiB)'), 'and(uinteger,min(1))', '8');
		test(form.Value, 'download_timeout_secs', _('Download timeout (seconds)'), 'and(uinteger,min(1))', '6');
		test(form.Value, 'tcp_timeout_ms', _('TCP timeout (ms)'), 'and(uinteger,min(1))', '1500');
		test(form.Flag, 'ip_sync_enabled', _('Automatically sync IP lists'), null, '1');
		choices(test(form.ListValue, 'ip_sync_mirror', _('IP list mirror'), null, 'github'),
			{ github: 'GitHub Raw', jsdelivr: 'jsDelivr', custom: _('Custom URL') });
		var base = test(form.Value, 'ip_sync_custom_base', _('Mirror base URL'));
		base.depends('ip_sync_mirror', 'custom'); base.rmempty = false; base.validate = webURL;
		var mac = s.taboption('ipv6', form.DynamicList, 'block_ipv6_mac', _('Fallback client MAC addresses'),
			_('Download AAAA records are already filtered in IPv4 mode. Leave this empty unless a client persistently uses cached IPv6 routes. The fallback restricts DHCPv6 and ICMPv6 and blocks forwarded IPv6 traffic for the listed clients.'));
		mac.datatype = 'macaddr';
		function updateRules() {
			var groups = {};
			Object.keys(groupSections).forEach(function(id) { groups[id] = groupSections[id].formvalue(id); });
			var rules = xb.rules(bind.formvalue('main') || '127.0.0.1', port.formvalue('main') || '5353', groups);
			Object.keys(blocks).forEach(function(name) { blocks[name].value = rules[name]; });
		}
		return m.render().then(function(node) {
			node.addEventListener('input', updateRules); node.addEventListener('change', updateRules);
			updateRules();
			return E('div', {}, [ xb.stylesheet(), node ]);
		});
	}
});
