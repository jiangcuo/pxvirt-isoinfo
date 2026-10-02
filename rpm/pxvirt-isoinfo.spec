# version and release are passed by the Makefile
%{!?pkgversion: %global pkgversion 0}
%{!?pkgrelease: %global pkgrelease 1}

# the binary is built and stripped by cargo
%global debug_package %{nil}

Name:           pxvirt-isoinfo
Version:        %{pkgversion}
Release:        %{pkgrelease}
Summary:        Detect the operating system on installation images
License:        AGPL-3.0-or-later
URL:            https://github.com/jiangcuo/pxvirt-isoinfo
Source0:        pxvirt-isoinfo
Source1:        rules.json

%description
Reads ISO 9660 (with Joliet and Rock Ridge) and UDF images without mounting
them and reports the operating system, version, architecture and installer as
JSON. For Windows images the editions in install.wim/install.esd are listed.

Linux distributions are detected with rules from
/usr/share/pxvirt-isoinfo/rules.json, local rules can be added in
/etc/pxvirt-isoinfo/rules.d/.

%prep

%build

%install
install -D -m 0755 %{SOURCE0} %{buildroot}%{_bindir}/pxvirt-isoinfo
install -D -m 0644 %{SOURCE1} %{buildroot}%{_datadir}/pxvirt-isoinfo/rules.json
install -d %{buildroot}%{_sysconfdir}/pxvirt-isoinfo/rules.d

%files
%{_bindir}/pxvirt-isoinfo
%{_datadir}/pxvirt-isoinfo
%dir %{_sysconfdir}/pxvirt-isoinfo
%dir %{_sysconfdir}/pxvirt-isoinfo/rules.d

%changelog
* Fri Oct 02 2026 Lierfang Support Team <itsupport@lierfang.com> - 0.1.0-1
- initial release
